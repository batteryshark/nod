//! Durable callback storage; policy validation and network delivery belong to the worker.
use super::now_string;
use crate::{auth::new_id, error::ApiError};
use sqlx::{Row, SqlitePool};

#[derive(Debug)]
pub struct CallbackClaim {
    pub delivery_id: String,
    pub event_id: String,
    pub request_revision: i64,
    pub url: String,
    pub payload_json: String,
    pub lease_token: String,
    pub attempts: i64,
}

/// Reserves an attempt before I/O. Includes blocked rows for periodic policy
/// revalidation; earlier undelivered revisions fence later callbacks.
pub async fn claim_due(pool: &SqlitePool) -> Result<Option<CallbackClaim>, ApiError> {
    let now = now_string();
    let until = (chrono::Utc::now() + chrono::Duration::seconds(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let token = new_id();
    let row = sqlx::query("UPDATE callback_deliveries SET state='sending',lease_token=?,lease_until=?,attempts=attempts+1,updated_at=? WHERE delivery_id=(SELECT c.delivery_id FROM callback_deliveries c WHERE c.state!='delivered' AND c.next_attempt_at<=? AND (c.lease_until IS NULL OR c.lease_until<=?) AND NOT EXISTS(SELECT 1 FROM callback_deliveries older WHERE older.request_id=c.request_id AND older.request_revision<c.request_revision AND older.state!='delivered') ORDER BY c.next_attempt_at,c.request_revision,c.delivery_id LIMIT 1) RETURNING *")
        .bind(&token).bind(until).bind(&now).bind(&now).bind(&now).fetch_optional(pool).await?;
    Ok(row.map(|r| CallbackClaim {
        delivery_id: r.get("delivery_id"),
        event_id: r.get("event_id"),
        request_revision: r.get("request_revision"),
        url: r.get("url"),
        payload_json: r.get("payload_json"),
        lease_token: token,
        attempts: r.get("attempts"),
    }))
}

pub async fn complete(
    pool: &SqlitePool,
    claim: &CallbackClaim,
    status: u16,
) -> Result<bool, ApiError> {
    if !(200..300).contains(&status) {
        return Err(ApiError::BadRequest("callback success requires 2xx".into()));
    }
    let now = now_string();
    Ok(sqlx::query("UPDATE callback_deliveries SET state='delivered',delivered_at=?,updated_at=?,last_status=?,last_error=NULL,lease_token=NULL,lease_until=NULL WHERE delivery_id=? AND state='sending' AND lease_token=? AND lease_until>?")
        .bind(&now).bind(&now).bind(i64::from(status)).bind(&claim.delivery_id).bind(&claim.lease_token).bind(&now).execute(pool).await?.rows_affected()==1)
}

/// No arbitrary error strings (which may contain private URLs/body). Retries all
/// failures without a max-attempt discard; blocked policy is periodically rechecked.
pub async fn retry(
    pool: &SqlitePool,
    claim: &CallbackClaim,
    status: Option<u16>,
    blocked_policy: bool,
) -> Result<bool, ApiError> {
    let now = now_string();
    let delay = if blocked_policy {
        300
    } else {
        (1_i64 << claim.attempts.clamp(0, 12)).min(3600)
    };
    let next = (chrono::Utc::now() + chrono::Duration::seconds(delay))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(sqlx::query("UPDATE callback_deliveries SET state=?,next_attempt_at=?,updated_at=?,last_status=?,last_error=?,lease_token=NULL,lease_until=NULL WHERE delivery_id=? AND state='sending' AND lease_token=? AND lease_until>?")
        .bind(if blocked_policy {"blocked_policy"} else {"retrying"}).bind(next).bind(&now).bind(status.map(i64::from)).bind(if blocked_policy {"policy_denied"} else {"delivery_failed"}).bind(&claim.delivery_id).bind(&claim.lease_token).bind(&now).execute(pool).await?.rows_affected()==1)
}
