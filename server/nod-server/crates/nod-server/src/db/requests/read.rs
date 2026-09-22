use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{Duration, Utc};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::rows::row_to_request;
use crate::{
    db::get_device,
    error::ApiError,
    models::{DecisionRequest, DecisionResolution, RequestStatus},
};

pub struct ListRequestsForDevice<'a> {
    pub device_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub include_cleared: bool,
    pub handled_limit: i64,
    pub retention_days: i64,
    pub search: Option<&'a str>,
    pub before: Option<&'a str>,
}

pub struct RequestPage {
    pub requests: Vec<DecisionRequest>,
    pub next_cursor: Option<String>,
}

pub async fn list_requests_for_device(
    pool: &SqlitePool,
    query: ListRequestsForDevice<'_>,
) -> Result<RequestPage, ApiError> {
    let device = get_device(pool, query.device_id).await?;
    let cutoff = (Utc::now() - Duration::days(query.retention_days.max(1)))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let cursor = query.before.map(decode_cursor).transpose()?;
    let handled_limit = query.handled_limit.clamp(0, 500);
    let mut transaction = pool.begin().await?;
    let mut rows = sqlx::query(r#"
        WITH visible AS (
            SELECT e.*, (e.status = 'pending' AND (e.decision_resolution = 'shared' OR eur.request_id IS NULL)) AS is_pending
            FROM requests e
            JOIN request_recipients er ON er.request_id = e.id AND er.user_id = ?1
            LEFT JOIN user_channel_subscriptions us ON us.channel_id = e.channel_id AND us.user_id = ?1
            LEFT JOIN user_channel_clears uc ON uc.channel_id = e.channel_id AND uc.user_id = ?1
            LEFT JOIN request_user_decisions eur ON eur.request_id = e.id AND eur.user_id = ?1
            WHERE (?2 IS NULL OR e.channel_id = ?2)
              AND (e.explicit_recipients = 1 OR us.subscribed = 1)
              AND (?3 = 1 OR uc.cleared_at IS NULL OR (e.status = 'pending' AND eur.request_id IS NULL) OR COALESCE(eur.resolved_at,e.resolved_at,e.updated_at) > uc.cleared_at)
              AND instr(lower(e.title || ' ' || e.summary || ' ' || e.body_markdown), lower(?4)) > 0
              AND (e.status = 'pending' OR COALESCE(e.resolved_at, e.updated_at) >= ?7)
        ), eligible AS (
            SELECT * FROM visible
            WHERE (is_pending = 1 AND ?5 IS NULL)
               OR (is_pending = 0 AND (?5 IS NULL OR (created_at, id) < (?5, ?6)))
        ), ranked AS (
            SELECT *, ROW_NUMBER() OVER(PARTITION BY is_pending ORDER BY created_at DESC, id DESC) AS history_rank
            FROM eligible
        )
        SELECT * FROM ranked WHERE is_pending = 1 OR history_rank <= ?8
        ORDER BY is_pending DESC, created_at DESC, id DESC
    "#)
        .bind(&device.user_id).bind(query.channel_id).bind(query.include_cleared)
        .bind(query.search.unwrap_or("").trim())
        .bind(cursor.as_ref().map(|value| value.0.as_str()))
        .bind(cursor.as_ref().map(|value| value.1.as_str()))
        .bind(cutoff).bind(if handled_limit > 0 { handled_limit + 1 } else { 0 })
        .fetch_all(&mut *transaction).await?;
    let history_count = rows
        .iter()
        .filter(|row| !row.get::<bool, _>("is_pending"))
        .count();
    let next_cursor = if history_count > handled_limit as usize {
        rows.pop();
        rows.last()
            .map(|row| encode_cursor(row.get("created_at"), row.get("id")))
            .transpose()?
    } else {
        None
    };
    let requests = super::rows::rows_to_requests(&mut transaction, rows)
        .await?
        .into_iter()
        .map(|request| project_request_for_user(request, &device.user_id))
        .collect();
    transaction.commit().await?;
    Ok(RequestPage {
        requests,
        next_cursor,
    })
}

fn encode_cursor(created_at: String, id: String) -> Result<String, ApiError> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&(created_at, id))?))
}

fn decode_cursor(value: &str) -> Result<(String, String), ApiError> {
    let invalid = || ApiError::BadRequest("invalid history cursor".to_string());
    if value.len() > 512 {
        return Err(invalid());
    }
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor: (String, String) = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    chrono::DateTime::parse_from_rfc3339(&cursor.0).map_err(|_| invalid())?;
    uuid::Uuid::parse_str(&cursor.1).map_err(|_| invalid())?;
    Ok(cursor)
}

pub async fn get_request(pool: &SqlitePool, request_id: &str) -> Result<DecisionRequest, ApiError> {
    let mut connection = pool.acquire().await?;
    get_request_on(&mut connection, request_id).await
}

pub(super) async fn get_request_on(
    connection: &mut SqliteConnection,
    request_id: &str,
) -> Result<DecisionRequest, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT id, channel_id, title, summary, body_markdown, fields_json, links_json,
            image_url, notification_json, dedupe_key, expires_at, status, created_at,
            updated_at, resolved_at, decision_json, callback_url, decision_resolution, recipient_salt
        FROM requests
        WHERE id = ?
        "#,
    )
    .bind(request_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(ApiError::NotFound)?;
    row_to_request(connection, row).await
}

pub async fn request_for_user(
    pool: &SqlitePool,
    request_id: &str,
    user_id: &str,
) -> Result<DecisionRequest, ApiError> {
    Ok(project_request_for_user(
        get_request(pool, request_id).await?,
        user_id,
    ))
}

fn project_request_for_user(mut request: DecisionRequest, user_id: &str) -> DecisionRequest {
    request.private_recipients = request.recipients.len() > 1;
    if request.decision_resolution == DecisionResolution::PerUser {
        // Device projections expose only the current user's decision state.
        if let Some(user_decision) = request
            .user_decisions
            .iter()
            .find(|decision| decision.user_id == user_id)
            .cloned()
        {
            request.status = RequestStatus::Resolved;
            request.resolved_at = Some(user_decision.decision.resolved_at);
            request.decision = Some(user_decision.decision);
        } else if matches!(request.status, RequestStatus::Resolved) {
            request.status = RequestStatus::Pending;
            request.resolved_at = None;
            request.decision = None;
        }
    }
    request.recipients.retain(|recipient| recipient == user_id);
    request
        .user_decisions
        .retain(|decision| decision.user_id == user_id);
    request
}

pub async fn request_visible_to_user(
    pool: &SqlitePool,
    request_id: &str,
    user_id: &str,
) -> Result<bool, ApiError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM request_recipients WHERE request_id = ? AND user_id = ?",
    )
    .bind(request_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

pub async fn request_created_by_issuer_token_id(
    pool: &SqlitePool,
    request_id: &str,
) -> Result<Option<String>, ApiError> {
    let row = sqlx::query("SELECT created_by_issuer_token_id FROM requests WHERE id = ?")
        .bind(request_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(row.get("created_by_issuer_token_id"))
}

pub async fn request_delivery_eligible(
    pool: &SqlitePool,
    request_id: &str,
    user_id: &str,
) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requests e JOIN request_recipients r ON r.request_id=e.id AND r.user_id=? LEFT JOIN user_channel_subscriptions s ON s.channel_id=e.channel_id AND s.user_id=r.user_id WHERE e.id=? AND (e.explicit_recipients=1 OR s.subscribed=1))")
        .bind(user_id).bind(request_id).fetch_one(pool).await?)
}

pub async fn recent_requests(pool: &SqlitePool) -> Result<Vec<DecisionRequest>, ApiError> {
    let mut transaction = pool.begin().await?;
    let rows = sqlx::query("SELECT * FROM requests ORDER BY created_at DESC,id DESC LIMIT 50")
        .fetch_all(&mut *transaction)
        .await?;
    let requests = super::rows::rows_to_requests(&mut transaction, rows).await?;
    transaction.commit().await?;
    Ok(requests)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn large_inbox_batches_related_rows_and_preserves_pagination_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", directory.path().join("nod.sqlite").display());
        config.data_dir = directory.path().to_path_buf();
        let pool = crate::db::connect(&config).await.unwrap();
        sqlx::query("INSERT INTO devices(id,user_id,name,platform,token_hash,last_seen_at,created_at) VALUES('device','owner','Reader','linux','hash',?,?)").bind(crate::db::now_string()).bind(crate::db::now_string()).execute(&pool).await.unwrap();
        sqlx::query("WITH RECURSIVE n(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM n WHERE value<5200) INSERT INTO requests(id,channel_id,title,summary,body_markdown,fields_json,links_json,status,created_at,updated_at,recipient_salt) SELECT printf('00000000-0000-0000-0000-%012d',value),'default','Searchable request','','Content','[]','[]',CASE WHEN value<=200 THEN 'pending' ELSE 'cancelled' END,?,?,'stable-test-salt' FROM n").bind(crate::db::now_string()).bind(crate::db::now_string()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO request_recipients(request_id,user_id,created_at) SELECT id,'owner',created_at FROM requests").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO request_options(request_id,option_id,kind,label,style,created_at) SELECT id,'approve','approve','Approve','default',created_at FROM requests").execute(&pool).await.unwrap();
        let mut samples = Vec::new();
        let mut cursor = None;
        for _ in 0..9 {
            let start = std::time::Instant::now();
            let page = list_requests_for_device(
                &pool,
                ListRequestsForDevice {
                    device_id: "device",
                    channel_id: Some("default"),
                    include_cleared: false,
                    handled_limit: 100,
                    retention_days: 7,
                    search: Some("SEARCHABLE"),
                    before: None,
                },
            )
            .await
            .unwrap();
            samples.push(start.elapsed());
            assert_eq!(page.requests.len(), 300);
            assert!(page
                .requests
                .iter()
                .all(|request| request.options.len() == 1 && request.signing.is_some()));
            cursor = page.next_cursor;
        }
        samples.sort();
        eprintln!(
            "5200-row inbox / 200 pending + 100 history: median {:?}, max {:?} (debug, 9 samples)",
            samples[4], samples[8]
        );
        let next = list_requests_for_device(
            &pool,
            ListRequestsForDevice {
                device_id: "device",
                channel_id: None,
                include_cleared: false,
                handled_limit: 100,
                retention_days: 7,
                search: Some("searchable"),
                before: cursor.as_deref(),
            },
        )
        .await
        .unwrap();
        assert_eq!(next.requests.len(), 100);
        assert!(next
            .requests
            .iter()
            .all(|request| request.status == RequestStatus::Cancelled));
    }
}
