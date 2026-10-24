use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::error::ApiError;

#[derive(Clone)]
pub(crate) struct PushJob {
    pub request_id: String,
    pub device_id: String,
    pub attempts: i64,
    pub claim_generation: i64,
}

pub(crate) const MAX_ATTEMPTS: i64 = 3;
// A task is cancelled after 60s, including preparation, sends and backoff.
pub(crate) const LEASE_SECONDS: i64 = 90;

#[derive(Serialize)]
pub(crate) struct PushDelivery {
    request_id: String,
    device_id: String,
    status: String,
    attempts: i64,
    updated_at: String,
    error: Option<String>,
}

pub(crate) async fn recover_pushes(pool: &SqlitePool) -> Result<(), ApiError> {
    // An interrupted send may have reached Apple. Reusing apns-collapse-id
    // minimizes duplicate visible notifications when recovering that send.
    // Never reset another worker's healthy lease, including at startup. NULL
    // leases are legacy interrupted jobs. Exhausted jobs must not cycle forever.
    sqlx::query("UPDATE push_deliveries SET status=CASE WHEN attempts>=3 THEN 'failed' ELSE 'queued' END, lease_until=NULL, error='interrupted push lease expired',updated_at=? WHERE (status='sending' AND (lease_until IS NULL OR lease_until<=unixepoch())) OR (status='queued' AND attempts>=3)")
        .bind(super::now_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn claim_pushes(pool: &SqlitePool, limit: i64) -> Result<Vec<PushJob>, ApiError> {
    let rows = sqlx::query("UPDATE push_deliveries SET status='sending',claim_generation=claim_generation+1,lease_until=unixepoch()+?,updated_at=? WHERE (request_id,device_id) IN (SELECT request_id,device_id FROM push_deliveries WHERE status='queued' AND attempts<3 ORDER BY updated_at LIMIT ?) RETURNING request_id,device_id,attempts,claim_generation")
        .bind(LEASE_SECONDS).bind(super::now_string()).bind(limit.max(0)).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| PushJob {
            request_id: row.get("request_id"),
            device_id: row.get("device_id"),
            attempts: row.get("attempts"),
            claim_generation: row.get("claim_generation"),
        })
        .collect())
}

pub(crate) struct PushProgress<'a> {
    pub job: &'a PushJob,
    pub status: &'a str,
    pub error: Option<&'a str>,
}

pub(crate) async fn update_push(
    pool: &SqlitePool,
    progress: PushProgress<'_>,
) -> Result<(), ApiError> {
    let result = sqlx::query("UPDATE push_deliveries SET status=?,updated_at=?,error=? WHERE request_id=? AND device_id=? AND status='sending' AND claim_generation=? AND lease_until>unixepoch() AND attempts=?")
        .bind(progress.status).bind(super::now_string()).bind(progress.error)
        .bind(&progress.job.request_id).bind(&progress.job.device_id).bind(progress.job.claim_generation).bind(progress.job.attempts).execute(pool).await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    Ok(())
}

/// Persist the attempt before any outward send, only while still owning this
/// generation. A failed fence or DB write must stop the task, not send anyway.
pub(crate) async fn begin_attempt(pool: &SqlitePool, job: &mut PushJob) -> Result<(), ApiError> {
    let result = sqlx::query("UPDATE push_deliveries SET attempts=attempts+1,updated_at=? WHERE request_id=? AND device_id=? AND status='sending' AND claim_generation=? AND lease_until>unixepoch() AND attempts=? AND attempts<3")
        .bind(super::now_string()).bind(&job.request_id).bind(&job.device_id)
        .bind(job.claim_generation).bind(job.attempts).execute(pool).await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    job.attempts += 1;
    Ok(())
}

/// Clear only the sent token, while this attempt still owns a live claim.
/// The ownership check and device mutation share one SQLite writer statement.
pub(crate) async fn clear_invalid_push_token(
    pool: &SqlitePool,
    job: &PushJob,
    token: Option<&str>,
) -> Result<bool, ApiError> {
    let result = sqlx::query("UPDATE devices SET push_token=NULL WHERE id=? AND push_token=? AND EXISTS (SELECT 1 FROM push_deliveries WHERE request_id=? AND device_id=? AND status='sending' AND claim_generation=? AND lease_until>unixepoch() AND attempts=?)")
        .bind(&job.device_id)
        .bind(token)
        .bind(&job.request_id)
        .bind(&job.device_id)
        .bind(job.claim_generation)
        .bind(job.attempts)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) async fn recent_push_deliveries(
    pool: &SqlitePool,
) -> Result<Vec<PushDelivery>, ApiError> {
    let rows = sqlx::query("SELECT request_id,device_id,status,attempts,updated_at,error FROM push_deliveries ORDER BY updated_at DESC LIMIT 100")
        .fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| PushDelivery {
            request_id: row.get("request_id"),
            device_id: row.get("device_id"),
            status: row.get("status"),
            attempts: row.get("attempts"),
            updated_at: row.get("updated_at"),
            error: row.get("error"),
        })
        .collect())
}
