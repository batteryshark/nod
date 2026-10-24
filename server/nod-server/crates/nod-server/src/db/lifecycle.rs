//! Immutable transactional integration journal. No network I/O belongs here.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::now_string;
use crate::{
    auth::new_id,
    error::ApiError,
    models::{Decision, DecisionRequest},
    views::CallbackPayload,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub session: String,
    pub directory: String,
    pub reference: Option<String>,
}

impl Origin {
    pub fn validate(&self) -> Result<(), ApiError> {
        let valid = |s: &str, max: usize| {
            !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
        };
        if !valid(&self.session, 256)
            || !self.session.starts_with("ses_")
            || self.session.len() <= 4
            || !valid(&self.directory, 4096)
            || !self.directory.starts_with('/')
            || self.reference.as_deref().is_some_and(|s| !valid(s, 512))
        {
            return Err(ApiError::BadRequest("invalid integration origin".into()));
        }
        Ok(())
    }
}

pub async fn origin_on(
    connection: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Origin>, ApiError> {
    Ok(
        sqlx::query("SELECT session,directory,reference FROM request_origins WHERE request_id=?")
            .bind(id)
            .fetch_optional(connection)
            .await?
            .map(|r| Origin {
                session: r.get("session"),
                directory: r.get("directory"),
                reference: r.get("reference"),
            }),
    )
}

/// Caller must hold the existing domain writer transaction and pass the exact accepted decision.
pub async fn append_lifecycle_on(
    connection: &mut SqliteConnection,
    request: &DecisionRequest,
    kind: &str,
    decision: Option<&Decision>,
) -> Result<(), ApiError> {
    let revision: i64 =
        sqlx::query_scalar("UPDATE requests SET revision=revision+1 WHERE id=? RETURNING revision")
            .bind(&request.id)
            .fetch_one(&mut *connection)
            .await?;
    let creator: Option<String> =
        sqlx::query_scalar("SELECT created_by_issuer_token_id FROM requests WHERE id=?")
            .bind(&request.id)
            .fetch_one(&mut *connection)
            .await?;
    let origin = origin_on(connection, &request.id).await?;
    let node: String = sqlx::query_scalar("SELECT value FROM integration_meta WHERE key='node_id'")
        .fetch_one(&mut *connection)
        .await?;
    let event_id = new_id();
    let now = now_string();
    let mut data = json!({"schema_version":1,"request_id":request.id,"revision":revision,"aggregate_status":request.status,"decision_resolution":request.decision_resolution});
    if decision.is_some() {
        data["decision_id"] = json!(event_id);
    }
    let envelope = json!({"specversion":"1.0","id":event_id,"source":format!("nod/{node}"),"type":kind,"subject":request.id,"time":now,"datacontenttype":"application/json","data":data});
    let detail = json!({"envelope":envelope,"channel_id":request.channel_id,"created_by_issuer_token_id":creator,"origin":origin,"decision":decision});
    sqlx::query("INSERT INTO lifecycle_events(event_id,request_id,channel_id,revision,kind,occurred_at,subject,envelope_json,private_detail_json,next_attempt_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(&event_id).bind(&request.id).bind(&request.channel_id).bind(revision).bind(kind).bind(&now).bind(format!("ev.{kind}"))
        .bind(serde_json::to_string(&envelope)?).bind(serde_json::to_string(&detail)?).bind(&now).execute(&mut *connection).await?;
    if decision.is_some() {
        if let Some(url) = &request.callback_url {
            sqlx::query("INSERT INTO callback_deliveries(delivery_id,event_id,request_id,request_revision,url,payload_json,state,next_attempt_at,updated_at) VALUES(?,?,?,?,?,?,'queued',?,?)")
                .bind(format!("{event_id}:callback")).bind(&event_id).bind(&request.id).bind(revision).bind(url)
                .bind(serde_json::to_string(&CallbackPayload::from_request(request))?).bind(&now).bind(&now).execute(&mut *connection).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ReplayEvent {
    pub seq: i64,
    pub envelope: Value,
}

pub async fn high_water(pool: &SqlitePool) -> Result<i64, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM lifecycle_events")
            .fetch_one(pool)
            .await?,
    )
}

/// Returns scanned rows, including saved channel for authorization after request pruning.
pub async fn scan(
    pool: &SqlitePool,
    after: i64,
    through: i64,
    limit: i64,
) -> Result<Vec<(ReplayEvent, String)>, ApiError> {
    let rows = sqlx::query("SELECT seq,envelope_json,channel_id FROM lifecycle_events WHERE seq>? AND seq<=? ORDER BY seq LIMIT ?")
        .bind(after).bind(through).bind(limit).fetch_all(pool).await?;
    rows.into_iter()
        .map(|r| {
            Ok((
                ReplayEvent {
                    seq: r.get("seq"),
                    envelope: serde_json::from_str(r.get("envelope_json"))?,
                },
                r.get("channel_id"),
            ))
        })
        .collect()
}

pub async fn detail(pool: &SqlitePool, id: &str) -> Result<(String, Value), ApiError> {
    let r =
        sqlx::query("SELECT channel_id,private_detail_json FROM lifecycle_events WHERE event_id=?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    Ok((
        r.get("channel_id"),
        serde_json::from_str(r.get("private_detail_json"))?,
    ))
}

#[derive(Debug)]
pub struct PublishClaim {
    pub event_id: String,
    pub subject: String,
    pub envelope_json: String,
    pub lease_token: String,
    pub attempts: i64,
}

/// Each claim durably reserves one attempt before network I/O. Expired leases are
/// reclaimed on every call; no startup reset or attempt ceiling.
pub async fn claim_due(pool: &SqlitePool) -> Result<Option<PublishClaim>, ApiError> {
    let now = now_string();
    let until = (chrono::Utc::now() + chrono::Duration::seconds(60))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let token = new_id();
    let r = sqlx::query("UPDATE lifecycle_events SET lease_token=?,lease_until=?,attempts=attempts+1 WHERE seq=(SELECT seq FROM lifecycle_events WHERE published_at IS NULL AND next_attempt_at<=? AND (lease_until IS NULL OR lease_until<=?) ORDER BY seq LIMIT 1) RETURNING event_id,subject,envelope_json,attempts")
        .bind(&token).bind(until).bind(&now).bind(&now).fetch_optional(pool).await?;
    Ok(r.map(|r| PublishClaim {
        event_id: r.get("event_id"),
        subject: r.get("subject"),
        envelope_json: r.get("envelope_json"),
        lease_token: token,
        attempts: r.get("attempts"),
    }))
}

pub async fn published(
    pool: &SqlitePool,
    claim: &PublishClaim,
    stream_seq: i64,
) -> Result<bool, ApiError> {
    let now = now_string();
    Ok(sqlx::query("UPDATE lifecycle_events SET published_at=?,stream_seq=?,lease_token=NULL,lease_until=NULL,last_error=NULL WHERE event_id=? AND lease_token=? AND lease_until>? AND published_at IS NULL")
        .bind(&now).bind(stream_seq).bind(&claim.event_id).bind(&claim.lease_token).bind(&now).execute(pool).await?.rows_affected()==1)
}

/// Deliberately stores only a caller-selected static diagnostic code, never transport text.
pub async fn retry(pool: &SqlitePool, claim: &PublishClaim) -> Result<bool, ApiError> {
    let now = now_string();
    let delay = (1_i64 << claim.attempts.clamp(0, 8)).min(300);
    let next = (chrono::Utc::now() + chrono::Duration::seconds(delay))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(sqlx::query("UPDATE lifecycle_events SET next_attempt_at=?,last_error='publish_failed',lease_token=NULL,lease_until=NULL WHERE event_id=? AND lease_token=? AND lease_until>? AND published_at IS NULL")
        .bind(next).bind(&claim.event_id).bind(&claim.lease_token).bind(now).execute(pool).await?.rows_affected()==1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queue_claims_are_fenced_and_callback_order_survives_expired_owners() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.data_dir = dir.path().join("data");
        config.database_url = format!("sqlite://{}", dir.path().join("queue.sqlite").display());
        let pool = crate::db::connect(&config).await.unwrap();
        for (id, revision) in [("a", 1), ("b", 2)] {
            sqlx::query("INSERT INTO lifecycle_events(event_id,request_id,channel_id,revision,kind,occurred_at,subject,envelope_json,private_detail_json,next_attempt_at) VALUES(?,'r','default',?,'answer',?,'ev.answer','{}','{}',?)")
                .bind(id).bind(revision).bind(now_string()).bind(now_string()).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO callback_deliveries(delivery_id,event_id,request_id,request_revision,url,payload_json,state,next_attempt_at,updated_at) VALUES(?,?,'r',?,'https://example.com','{}','queued',?,?)")
                .bind(id).bind(id).bind(revision).bind(now_string()).bind(now_string()).execute(&pool).await.unwrap();
        }
        let old = claim_due(&pool).await.unwrap().unwrap();
        sqlx::query("UPDATE lifecycle_events SET lease_until='2000-01-01' WHERE event_id=?")
            .bind(&old.event_id)
            .execute(&pool)
            .await
            .unwrap();
        let fresh = claim_due(&pool).await.unwrap().unwrap();
        assert_eq!(old.event_id, fresh.event_id);
        assert_eq!(old.envelope_json, fresh.envelope_json);
        assert_eq!(fresh.attempts, 2);
        assert!(!published(&pool, &old, 1).await.unwrap());
        assert!(!retry(&pool, &old).await.unwrap());
        assert!(published(&pool, &fresh, 1).await.unwrap());
        use crate::db::callback_deliveries as cb;
        let old = cb::claim_due(&pool).await.unwrap().unwrap();
        assert_eq!(old.request_revision, 1);
        assert!(cb::claim_due(&pool).await.unwrap().is_none());
        sqlx::query("UPDATE callback_deliveries SET lease_until='2000-01-01' WHERE delivery_id=?")
            .bind(&old.delivery_id)
            .execute(&pool)
            .await
            .unwrap();
        let fresh = cb::claim_due(&pool).await.unwrap().unwrap();
        assert_eq!(fresh.delivery_id, old.delivery_id);
        assert!(!cb::complete(&pool, &old, 204).await.unwrap());
        assert!(!cb::retry(&pool, &old, None, true).await.unwrap());
        assert!(cb::complete(&pool, &fresh, 204).await.unwrap());
        assert_eq!(
            cb::claim_due(&pool)
                .await
                .unwrap()
                .unwrap()
                .request_revision,
            2
        );
    }
}
