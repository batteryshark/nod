use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;

use crate::{audit::AuditHealth, auth, db, error::ApiError, state::AppState};

#[derive(Serialize)]
pub(super) struct ActivityResponse {
    requests: Vec<nod_proto::Request>,
    deliveries: Vec<db::push_deliveries::PushDelivery>,
    audit: AuditHealth,
}

pub(super) async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ActivityResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let requests = db::recent_requests(&state.pool)
        .await?
        .into_iter()
        .map(|request| request.to_wire())
        .collect();
    Ok(Json(ActivityResponse {
        requests,
        deliveries: db::push_deliveries::recent_push_deliveries(&state.pool).await?,
        audit: state.audit.health().await,
    }))
}
