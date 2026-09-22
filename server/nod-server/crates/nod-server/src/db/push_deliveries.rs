use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::error::ApiError;

#[derive(Clone)]
pub(crate) struct PushJob {
    pub request_id: String,
    pub device_id: String,
    pub attempts: i64,
}

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
    sqlx::query("UPDATE push_deliveries SET status='queued',updated_at=? WHERE status='sending'")
        .bind(super::now_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn claim_pushes(pool: &SqlitePool, limit: i64) -> Result<Vec<PushJob>, ApiError> {
    let rows = sqlx::query("UPDATE push_deliveries SET status='sending',updated_at=? WHERE (request_id,device_id) IN (SELECT request_id,device_id FROM push_deliveries WHERE status='queued' ORDER BY updated_at LIMIT ?) RETURNING request_id,device_id,attempts")
        .bind(super::now_string()).bind(limit).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| PushJob {
            request_id: row.get("request_id"),
            device_id: row.get("device_id"),
            attempts: row.get("attempts"),
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
    sqlx::query("UPDATE push_deliveries SET status=?,attempts=?,updated_at=?,error=? WHERE request_id=? AND device_id=?")
        .bind(progress.status).bind(progress.job.attempts).bind(super::now_string()).bind(progress.error)
        .bind(&progress.job.request_id).bind(&progress.job.device_id).execute(pool).await?;
    Ok(())
}

pub(crate) async fn clear_invalid_push_token(
    pool: &SqlitePool,
    device_id: &str,
    token: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE devices SET push_token=NULL WHERE id=? AND push_token=?")
        .bind(device_id)
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
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
