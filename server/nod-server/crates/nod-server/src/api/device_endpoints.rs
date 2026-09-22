use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use super::responses::{ChannelsResponse, OkResponse, UserDeviceResponse, UserDevicesResponse};
use crate::{
    auth, db,
    error::ApiError,
    models::{
        CurrentUserResponse, EnrollDeviceRequest, EnrollDeviceResponse,
        UpdateDevicePreferencesRequest, UpdatePushTokenRequest, UpdateSubscriptionRequest,
        UpdateUserDeviceRequest,
    },
    services,
    state::AppState,
};

pub(super) async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ChannelsResponse>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    let channels = match principal {
        auth::Principal::Device(device) => {
            db::list_channels_for_device(&state.pool, &device.id).await?
        }
        _ => db::list_channels(&state.pool).await?,
    };
    Ok(Json(ChannelsResponse { channels }))
}

pub(super) async fn enroll_device(
    State(state): State<AppState>,
    Json(req): Json<EnrollDeviceRequest>,
) -> Result<Json<EnrollDeviceResponse>, ApiError> {
    Ok(Json(services::devices::enroll(&state, req).await?))
}

pub(super) async fn current_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentUserResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let user = db::get_user(&state.pool, &device.user_id).await?;
    let devices = db::list_user_devices(&state.pool, &device.user_id, &device.id).await?;
    let current_device = devices
        .into_iter()
        .find(|user_device| user_device.id == device.id)
        .ok_or(ApiError::NotFound)?;
    Ok(Json(CurrentUserResponse {
        user,
        current_device,
        notification_delivery: state.delivery_for_device(&device),
    }))
}

pub(super) async fn list_user_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserDevicesResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let devices = db::list_user_devices(&state.pool, &device.user_id, &device.id).await?;
    Ok(Json(UserDevicesResponse { devices }))
}

pub(super) async fn update_user_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(req): Json<UpdateUserDeviceRequest>,
) -> Result<Json<UserDeviceResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let updated = services::devices::rename_user_device(&state, &device, &device_id, req).await?;
    Ok(Json(UserDeviceResponse { device: updated }))
}

pub(super) async fn revoke_user_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    services::devices::revoke_user_device(&state, &device, &device_id).await?;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn update_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdatePushTokenRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    services::devices::update_push_token(&state, &device, req).await?;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn update_device_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateDevicePreferencesRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    services::devices::update_preferences(&state, &device, req).await?;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn update_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    services::devices::update_subscription(&state, &device, &channel_id, req).await?;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn clear_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    services::devices::clear_channel(&state, &device, &channel_id).await?;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn update_notification_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(preferences): Json<nod_proto::DeviceNotificationPreferences>,
) -> Result<Json<OkResponse>, ApiError> {
    let device = auth::require_device(&headers, &state.pool).await?;
    let preferences =
        db::update_notification_preferences(&state.pool, &device.id, preferences).await?;
    state
        .audit
        .record(
            "device.notification_preferences_updated",
            &serde_json::json!({"device_id":device.id,"preferences":preferences}),
        )
        .await;
    let _ = state.sync.send(crate::sync::targeted_envelope(
        "device_preferences_updated",
        serde_json::json!({"device_id":device.id}),
        vec![device.user_id],
    ));
    Ok(Json(OkResponse::ok()))
}
