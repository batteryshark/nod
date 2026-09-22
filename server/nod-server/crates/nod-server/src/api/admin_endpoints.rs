use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde_json::json;

use super::requests::CreateRequestRequest;
use super::responses::{
    AdminDevicesResponse, AdminIssuerTokensResponse, AdminUsersResponse, ChannelResponse,
    ChannelsResponse, CreateRequestResponse, EnrollmentResponse, IssuerTokenResponse, OkResponse,
    UserResponse,
};
use crate::{
    auth, db,
    error::ApiError,
    models::{
        AdminApnsRelaySettings, AdminAppleAppAttestSettings, AdminDeviceAttestationSettings,
        AdminSettings, AdminSummary, CreateChannelRequest, CreateEnrollmentCodeRequest,
        CreateIssuerTokenRequest, CreateUserRequest, UpdateSubscriptionRequest, UpdateUserRequest,
        UpdateUserSubscriptionsRequest,
    },
    services,
    state::AppState,
    sync,
};

pub(super) async fn admin_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminSummary>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let counts = db::admin_counts(&state.pool).await?;
    Ok(Json(AdminSummary {
        users: counts.users,
        channels: counts.channels,
        devices: counts.devices,
        active_issuer_tokens: counts.active_issuer_tokens,
        pending_requests: counts.pending_requests,
        notification_delivery_mode: state.notification_delivery.mode.as_str().to_string(),
        push_route: state.push_route.map(|route| route.as_str().to_string()),
        retention_days: state.config.retention_days,
    }))
}

pub(super) async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminSettings>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let config = &state.config;
    let apns_relay = &config.notifications.apns_relay;
    let app_attest = &config.device_attestation.apple_app_attest;
    let apns_relay_url = apns_relay
        .url
        .as_ref()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned);
    Ok(Json(AdminSettings {
        notification_delivery_mode: state.notification_delivery.mode.as_str().to_string(),
        push_route: state.push_route.map(|route| route.as_str().to_string()),
        retention_days: config.retention_days,
        apns_relay: AdminApnsRelaySettings {
            client_enabled: apns_relay.client_enabled(),
            url: apns_relay_url,
            native_app_id: apns_relay.native_app_id.clone(),
            client_cert_configured: apns_relay.tls.client_cert_configured(),
            client_key_configured: apns_relay.tls.client_key_configured(),
            ca_cert_configured: apns_relay.tls.ca_cert_configured(),
        },
        device_attestation: AdminDeviceAttestationSettings {
            apple_app_attest: AdminAppleAppAttestSettings {
                mode: app_attest.mode.as_str().to_string(),
                team_id_configured: app_attest.team_id_configured(),
                bundle_ids: app_attest.normalized_bundle_ids(),
                environment: app_attest.environment.as_str().to_string(),
            },
        },
    }))
}

pub(super) async fn list_admin_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ChannelsResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let channels = db::list_channels(&state.pool).await?;
    Ok(Json(ChannelsResponse { channels }))
}

pub(super) async fn list_admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminUsersResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let users = db::list_users_for_admin(&state.pool).await?;
    Ok(Json(AdminUsersResponse { users }))
}

pub(super) async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let user = db::create_user(&state.pool, req).await?;
    state.audit.record("user.upserted", &user).await;
    let _ = state.sync.send(sync::device_update(
        "user_updated",
        json!({ "user_id": &user.id }),
    ));
    Ok(Json(UserResponse { user }))
}

pub(super) async fn update_admin_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let user = db::update_user(&state.pool, &user_id, req).await?;
    state.audit.record("user.updated", &user).await;
    let _ = state.sync.send(sync::device_update(
        "user_updated",
        json!({ "user_id": &user.id }),
    ));
    Ok(Json(UserResponse { user }))
}

pub(super) async fn delete_admin_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    db::delete_user(&state.pool, &user_id).await?;
    state
        .audit
        .record("user.deleted", &json!({ "user_id": &user_id }))
        .await;
    let _ = state.sync.send(sync::device_update(
        "user_deleted",
        json!({ "user_id": &user_id }),
    ));
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn create_enrollment_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<CreateEnrollmentCodeRequest>,
) -> Result<Json<EnrollmentResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let enrollment = db::create_enrollment_code(&state.pool, &user_id, req).await?;
    state
        .audit
        .record(
            "enrollment_code.created",
            &json!({ "user_id": &user_id, "expires_at": enrollment.expires_at }),
        )
        .await;
    Ok(Json(enrollment))
}

pub(super) async fn list_admin_user_subscriptions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<ChannelsResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let channels = db::list_user_subscriptions_for_admin(&state.pool, &user_id).await?;
    Ok(Json(ChannelsResponse { channels }))
}

pub(super) async fn update_admin_user_subscriptions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserSubscriptionsRequest>,
) -> Result<Json<ChannelsResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let updates = req.updates;
    let channels = db::update_user_subscriptions_for_admin(&state.pool, &user_id, &updates).await?;
    if !updates.is_empty() {
        state
            .audit
            .record(
                "user.subscription_updated",
                &json!({ "user_id": &user_id, "updates": &updates }),
            )
            .await;
        let _ = state.sync.send(sync::targeted_envelope(
            "subscription_updated",
            json!({ "user_id": &user_id, "updates": &updates }),
            vec![user_id.clone()],
        ));
    }
    Ok(Json(ChannelsResponse { channels }))
}

pub(super) async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateChannelRequest>,
) -> Result<Json<ChannelResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let channel = db::create_channel(&state.pool, req).await?;
    state.audit.record("channel.upserted", &channel).await;
    let _ = state
        .sync
        .send(sync::channel_update("channel_updated", &channel));
    Ok(Json(ChannelResponse { channel }))
}

pub(super) async fn delete_admin_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    db::delete_channel(&state.pool, &channel_id).await?;
    state
        .audit
        .record("channel.deleted", &json!({ "channel_id": &channel_id }))
        .await;
    let _ = state.sync.send(sync::device_update(
        "channel_deleted",
        json!({ "channel_id": &channel_id }),
    ));
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn list_admin_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminDevicesResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let devices = db::list_devices_for_admin(&state.pool).await?;
    Ok(Json(AdminDevicesResponse { devices }))
}

pub(super) async fn delete_admin_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    db::delete_device(&state.pool, &device_id).await?;
    state
        .audit
        .record("device.revoked", &json!({ "device_id": &device_id }))
        .await;
    let _ = state.sync.send(sync::device_update(
        "device_revoked",
        json!({ "device_id": &device_id }),
    ));
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn update_admin_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, channel_id)): Path<(String, String)>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    db::get_device(&state.pool, &device_id).await?;
    db::set_subscription(&state.pool, &device_id, &channel_id, req.subscribed).await?;
    state
        .audit
        .record(
            "device.subscription_updated",
            &json!({
                "device_id": &device_id,
                "channel_id": &channel_id,
                "subscribed": req.subscribed
            }),
        )
        .await;
    let _ = state.sync.send(sync::device_update(
        "subscription_updated",
        json!({ "device_id": &device_id, "channel_id": &channel_id, "subscribed": req.subscribed }),
    ));
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn list_admin_issuer_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminIssuerTokensResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let tokens = db::list_issuer_tokens_for_admin(&state.pool).await?;
    Ok(Json(AdminIssuerTokensResponse { tokens }))
}

pub(super) async fn create_issuer_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateIssuerTokenRequest>,
) -> Result<Json<IssuerTokenResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let response = db::create_issuer_token(&state.pool, req).await?;
    state
        .audit
        .record(
            "issuer_token.created",
            &json!({ "id": response.id, "scopes": response.scopes }),
        )
        .await;
    Ok(Json(response))
}

pub(super) async fn revoke_admin_issuer_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token_id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    db::revoke_issuer_token(&state.pool, &token_id).await?;
    state
        .audit
        .record("issuer_token.revoked", &json!({ "id": &token_id }))
        .await;
    Ok(Json(OkResponse::ok()))
}

pub(super) async fn create_admin_test_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRequestRequest>,
) -> Result<Json<CreateRequestResponse>, ApiError> {
    auth::require_admin(&headers, state.config.admin_token()).await?;
    let idempotency_key = req.idempotency_key.clone();
    let response = services::requests::create(
        &state,
        req.into(),
        db::CreateRequestMetadata {
            created_by_issuer_token_id: None,
            idempotency_key: idempotency_key.as_deref(),
        },
    )
    .await?;
    Ok(Json(CreateRequestResponse::from_created_request(&response)))
}
