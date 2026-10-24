use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;

use crate::{audit::AuditHealth, auth, db, error::ApiError, state::AppState};

#[derive(Serialize)]
pub(super) struct ActivityResponse {
    requests: Vec<nod_proto::Request>,
    deliveries: Vec<db::push_deliveries::PushDelivery>,
    audit: AuditHealth,
    integrations: IntegrationActivity,
}

#[derive(Serialize)]
struct IntegrationActivity {
    publisher_status: &'static str,
    lifecycle_total: i64,
    lifecycle_pending: i64,
    lifecycle_retrying: i64,
    lifecycle_max_attempts: i64,
    lifecycle_oldest_pending_at: Option<String>,
    callbacks_queued: i64,
    callbacks_retrying: i64,
    callbacks_blocked_policy: i64,
    callbacks_sending: i64,
    callbacks_delivered: i64,
    callbacks_max_attempts: i64,
    callbacks_oldest_pending_at: Option<String>,
}

pub(super) async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ActivityResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    use sqlx::Row;
    let events = sqlx::query("SELECT count(*) AS total, coalesce(max(attempts),0) AS max_attempts, count(CASE WHEN published_at IS NULL THEN 1 END) AS pending, count(CASE WHEN published_at IS NULL AND attempts>0 THEN 1 END) AS retrying, min(CASE WHEN published_at IS NULL THEN occurred_at END) AS oldest FROM lifecycle_events")
        .fetch_one(&state.pool).await?;
    let callbacks = sqlx::query("SELECT coalesce(max(attempts),0) AS max_attempts, count(CASE WHEN state='queued' THEN 1 END) AS queued, count(CASE WHEN state='retrying' THEN 1 END) AS retrying, count(CASE WHEN state='blocked_policy' THEN 1 END) AS blocked, count(CASE WHEN state='sending' THEN 1 END) AS sending, count(CASE WHEN state='delivered' THEN 1 END) AS delivered, min(CASE WHEN state!='delivered' THEN (SELECT occurred_at FROM lifecycle_events e WHERE e.event_id=callback_deliveries.event_id) END) AS oldest FROM callback_deliveries")
        .fetch_one(&state.pool).await?;
    let integrations = IntegrationActivity {
        publisher_status: state.publisher_health.status(),
        lifecycle_total: events.get("total"),
        lifecycle_pending: events.get("pending"),
        lifecycle_retrying: events.get("retrying"),
        lifecycle_max_attempts: events.get("max_attempts"),
        lifecycle_oldest_pending_at: events.get("oldest"),
        callbacks_queued: callbacks.get("queued"),
        callbacks_retrying: callbacks.get("retrying"),
        callbacks_blocked_policy: callbacks.get("blocked"),
        callbacks_sending: callbacks.get("sending"),
        callbacks_delivered: callbacks.get("delivered"),
        callbacks_max_attempts: callbacks.get("max_attempts"),
        callbacks_oldest_pending_at: callbacks.get("oldest"),
    };
    let requests = db::recent_requests(&state.pool)
        .await?
        .into_iter()
        .map(|request| request.to_wire())
        .collect();
    Ok(Json(ActivityResponse {
        requests,
        deliveries: db::push_deliveries::recent_push_deliveries(&state.pool).await?,
        audit: state.audit.health().await,
        integrations,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn activity_reports_durable_backlog_without_private_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test-admin");
        config.database_url = format!("sqlite://{}", dir.path().join("activity.sqlite").display());
        config.data_dir = dir.path().into();
        config.callback_allowed_origins = Some(vec![]);
        let state = AppState::new(config).await.unwrap();
        sqlx::query("INSERT INTO lifecycle_events(event_id,request_id,channel_id,revision,kind,occurred_at,subject,envelope_json,private_detail_json,next_attempt_at) VALUES('e','r','default',1,'answer','2020-01-01','ev.answer','{}','private-content','2020-01-01')")
            .execute(&state.pool).await.unwrap();
        sqlx::query("INSERT INTO callback_deliveries(delivery_id,event_id,request_id,request_revision,url,payload_json,state,next_attempt_at,updated_at) VALUES('d','e','r',1,'http://127.0.0.1:1/?private-token','private-body','blocked_policy','2100-01-01','2020-01-01')")
            .execute(&state.pool).await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-admin".parse().unwrap());
        let Json(response) = activity(State(state), headers).await.unwrap();
        assert_eq!(response.integrations.publisher_status, "disabled");
        assert_eq!(response.integrations.lifecycle_pending, 1);
        assert_eq!(response.integrations.callbacks_blocked_policy, 1);
        assert_eq!(
            response.integrations.callbacks_oldest_pending_at.as_deref(),
            Some("2020-01-01")
        );
        let json = serde_json::to_string(&response).unwrap();
        for secret in [
            "private-content",
            "private-token",
            "private-body",
            "127.0.0.1",
            "test-admin",
        ] {
            assert!(!json.contains(secret));
        }
        assert!(json.contains("audit")); // JSONL health remains a separate boundary.
    }
}
