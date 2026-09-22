use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::responses::{
    CreateRequestResponse, RequestDecisionResponse, RequestResponse, RequestsResponse,
};
use crate::{
    auth, db,
    error::ApiError,
    models::{
        CardField, CardLink, CreateDecisionRequest, DecisionResolution, RequestNotification,
        RequestOption, SubmitDecisionRequest,
    },
    services,
    state::AppState,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateRequestRequest {
    #[serde(default = "default_channel")]
    channel_id: String,
    #[serde(default)]
    pub(super) idempotency_key: Option<String>,
    #[serde(default)]
    recipients: Option<Vec<String>>,
    #[serde(default)]
    decision_resolution: Option<DecisionResolution>,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    body_markdown: String,
    #[serde(default)]
    fields: Vec<CardField>,
    #[serde(default)]
    links: Vec<CardLink>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    notification: RequestNotification,
    #[serde(default)]
    dedupe_key: Option<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    options: Vec<RequestOption>,
    #[serde(default)]
    callback_url: Option<String>,
    #[serde(default)]
    template_id: Option<String>,
    #[serde(default)]
    template_version: Option<String>,
    #[serde(default)]
    variables: Option<Value>,
}

fn default_channel() -> String {
    "default".to_string()
}

impl From<CreateRequestRequest> for CreateDecisionRequest {
    fn from(req: CreateRequestRequest) -> Self {
        // Accept template metadata while storing only the rendered request snapshot.
        let _template_snapshot = (&req.template_id, &req.template_version, &req.variables);
        Self {
            channel_id: req.channel_id,
            recipients: req.recipients,
            decision_resolution: req.decision_resolution,
            title: req.title,
            summary: req.summary,
            body_markdown: req.body_markdown,
            fields: req.fields,
            links: req.links,
            image_url: req.image_url,
            notification: req.notification,
            dedupe_key: req.dedupe_key,
            expires_at: req.expires_at,
            options: req.options,
            callback_url: req.callback_url,
        }
    }
}

pub(super) async fn create_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRequestRequest>,
) -> Result<Json<CreateRequestResponse>, ApiError> {
    let channel_id = req.channel_id.clone();
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    auth::require_request_write(&principal, &channel_id)?;
    let created_by_issuer_token_id = match &principal {
        auth::Principal::Issuer(token) => Some(token.id.as_str()),
        _ => None,
    };
    let idempotency_key = req.idempotency_key.clone();
    let response = services::requests::create(
        &state,
        req.into(),
        db::CreateRequestMetadata {
            created_by_issuer_token_id,
            idempotency_key: idempotency_key.as_deref(),
        },
    )
    .await?;
    Ok(Json(CreateRequestResponse::from_created_request(&response)))
}

pub(super) async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListRequestsQuery>,
) -> Result<Json<RequestsResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let requests = db::list_requests_for_device(
        &state.pool,
        db::ListRequestsForDevice {
            device_id: &device.id,
            channel_id: query.channel_id.as_deref(),
            include_cleared: query.include_cleared.unwrap_or(false),
            handled_limit: query.limit.unwrap_or(100),
            retention_days: state.config.retention_days,
            search: query.search.as_deref(),
            before: query.before.as_deref(),
        },
    )
    .await?;
    Ok(Json(RequestsResponse::from_page(
        requests.requests,
        requests.next_cursor,
    )))
}

pub(super) async fn get_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<RequestResponse>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    let request =
        services::requests::request_for_principal(&state, &principal, &request_id).await?;
    Ok(Json(RequestResponse::from_request(&request)))
}

pub(super) async fn cancel_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<RequestResponse>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    let request = db::get_request(&state.pool, &request_id).await?;
    auth::require_request_cancel(&principal, &request.channel_id)?;
    if let auth::Principal::Issuer(token) = &principal {
        let creator = db::request_created_by_issuer_token_id(&state.pool, &request_id).await?;
        if creator.as_deref() != Some(token.id.as_str()) {
            return Err(ApiError::Forbidden);
        }
    }
    let request = services::requests::cancel(&state, &request_id).await?;
    Ok(Json(RequestResponse::from_request(&request)))
}

pub(super) async fn get_request_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<RequestDecisionResponse>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    let request =
        services::requests::request_for_principal(&state, &principal, &request_id).await?;
    Ok(Json(RequestDecisionResponse::from_request(&request)))
}

pub(super) async fn wait_for_request_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Query(query): Query<WaitQuery>,
) -> Result<Json<RequestDecisionResponse>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    let initial_request = db::get_request(&state.pool, &request_id).await?;
    let device_user_id = match &principal {
        auth::Principal::Device(device) => {
            if !db::request_visible_to_user(&state.pool, &request_id, &device.user_id).await? {
                return Err(ApiError::Forbidden);
            }
            Some(device.user_id.clone())
        }
        _ => {
            auth::require_request_read(&principal, &initial_request.channel_id)?;
            None
        }
    };
    let wait_for = Duration::from_secs(query.timeout_seconds.unwrap_or(55).clamp(1, 60));
    let waited = services::requests::wait_for_decision(
        &state,
        &request_id,
        device_user_id.as_deref(),
        wait_for,
    )
    .await?;
    let mut response = RequestDecisionResponse::from_request(&waited.request);
    if waited.timed_out {
        response.mark_timed_out();
    }
    Ok(Json(response))
}

pub(super) async fn submit_option(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((request_id, option_id)): Path<(String, String)>,
    Json(req): Json<SubmitDecisionRequest>,
) -> Result<Json<RequestResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let request =
        services::requests::record_decision(&state, &device, &request_id, &option_id, req).await?;
    Ok(Json(RequestResponse::from_request(&request)))
}

#[derive(Debug, Deserialize)]
pub(super) struct ListRequestsQuery {
    channel_id: Option<String>,
    include_cleared: Option<bool>,
    limit: Option<i64>,
    search: Option<String>,
    before: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WaitQuery {
    timeout_seconds: Option<u64>,
}
