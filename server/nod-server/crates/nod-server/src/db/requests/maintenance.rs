use chrono::{Duration, Utc};
use sqlx::{Row, SqlitePool};

use super::read::get_request;
use crate::{
    db::now_string,
    error::ApiError,
    models::{DecisionRequest, RequestStatus},
};

pub async fn cancel_request(
    pool: &SqlitePool,
    request_id: &str,
) -> Result<DecisionRequest, ApiError> {
    let request = get_request(pool, request_id).await?;
    if !matches!(request.status, RequestStatus::Pending) {
        return Err(ApiError::Conflict(
            "request is no longer pending".to_string(),
        ));
    }

    let now = now_string();
    let updated = sqlx::query(
        "UPDATE requests SET status = 'cancelled', updated_at = ? WHERE id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(request_id)
    .execute(pool)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "request is no longer pending".to_string(),
        ));
    }

    get_request(pool, request_id).await
}

pub async fn expire_due_requests(pool: &SqlitePool) -> Result<Vec<DecisionRequest>, ApiError> {
    let now = now_string();
    let rows = sqlx::query(
        "UPDATE requests SET status = 'expired', updated_at = ? WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ? RETURNING id",
    )
    .bind(&now)
    .bind(&now)
    .fetch_all(pool)
    .await?;
    let ids: Vec<String> = rows.into_iter().map(|row| row.get("id")).collect();
    let mut expired = Vec::new();
    for id in ids {
        expired.push(get_request(pool, &id).await?);
    }
    Ok(expired)
}

pub async fn prune_retention(pool: &SqlitePool, retention_days: i64) -> Result<u64, ApiError> {
    let cutoff = (Utc::now() - Duration::days(retention_days.max(1)))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let deleted = sqlx::query(
        "DELETE FROM requests WHERE status != 'pending' AND COALESCE(resolved_at, updated_at) < ?",
    )
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(deleted.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn retention_preserves_pending_and_recently_handled_old_requests() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", directory.path().join("nod.sqlite").display());
        config.data_dir = directory.path().to_path_buf();
        let pool = crate::db::connect(&config).await.unwrap();
        for status in ["pending", "resolved", "cancelled", "expired"] {
            sqlx::query("INSERT INTO requests(id,channel_id,title,summary,body_markdown,fields_json,links_json,status,created_at,updated_at) VALUES(?,'default','Old','','','[]','[]',?,'2000-01-01T00:00:00.000Z','2000-01-01T00:00:00.000Z')")
                .bind(status).bind(status).execute(&pool).await.unwrap();
        }
        sqlx::query("UPDATE requests SET updated_at=? WHERE status='resolved'")
            .bind(now_string())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(prune_retention(&pool, 1).await.unwrap(), 2);
        let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM requests ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, vec!["pending", "resolved"]);
    }
}
