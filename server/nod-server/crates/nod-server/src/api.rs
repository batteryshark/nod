use axum::{
    routing::{delete, get, post, put},
    Json, Router,
};
use tower_http::cors::{Any, CorsLayer};

use self::responses::HealthResponse;
use crate::{admin, state::AppState};

mod activity;
mod admin_endpoints;
mod device_endpoints;
mod requests;
mod responses;
mod sync_socket;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/admin", get(admin::admin_page))
        .route("/admin/", get(admin::admin_page))
        .route(
            "/admin/session",
            post(admin::create_admin_session).delete(admin::delete_admin_session),
        )
        .route("/health", get(health))
        .route("/api/v1/channels", get(device_endpoints::list_channels))
        .route(
            "/api/v1/admin/channels",
            get(admin_endpoints::list_admin_channels).post(admin_endpoints::create_channel),
        )
        .route(
            "/api/v1/admin/channels/{channel_id}",
            delete(admin_endpoints::delete_admin_channel),
        )
        .route(
            "/api/v1/admin/users",
            get(admin_endpoints::list_admin_users).post(admin_endpoints::create_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}",
            put(admin_endpoints::update_admin_user).delete(admin_endpoints::delete_admin_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}/enrollment-codes",
            post(admin_endpoints::create_enrollment_code),
        )
        .route(
            "/api/v1/admin/users/{user_id}/subscriptions",
            get(admin_endpoints::list_admin_user_subscriptions)
                .put(admin_endpoints::update_admin_user_subscriptions),
        )
        .route(
            "/api/v1/admin/devices",
            get(admin_endpoints::list_admin_devices),
        )
        .route(
            "/api/v1/admin/devices/{device_id}",
            delete(admin_endpoints::delete_admin_device),
        )
        .route(
            "/api/v1/admin/devices/{device_id}/subscriptions/{channel_id}",
            put(admin_endpoints::update_admin_subscription),
        )
        .route(
            "/api/v1/admin/issuer-tokens",
            get(admin_endpoints::list_admin_issuer_tokens)
                .post(admin_endpoints::create_issuer_token),
        )
        .route(
            "/api/v1/admin/issuer-tokens/{token_id}",
            delete(admin_endpoints::revoke_admin_issuer_token),
        )
        .route(
            "/api/v1/admin/test-requests",
            post(admin_endpoints::create_admin_test_request),
        )
        .route("/api/v1/admin/activity", get(activity::activity))
        .route("/api/v1/admin/summary", get(admin_endpoints::admin_summary))
        .route(
            "/api/v1/admin/settings",
            get(admin_endpoints::admin_settings),
        )
        .route("/api/v1/enroll", post(device_endpoints::enroll_device))
        .route("/api/v1/users/me", get(device_endpoints::current_user))
        .route(
            "/api/v1/users/me/devices",
            get(device_endpoints::list_user_devices),
        )
        .route(
            "/api/v1/users/me/devices/{device_id}",
            put(device_endpoints::update_user_device).delete(device_endpoints::revoke_user_device),
        )
        .route(
            "/api/v1/devices/me/push-token",
            put(device_endpoints::update_push_token),
        )
        .route(
            "/api/v1/devices/me/notification-preferences",
            put(device_endpoints::update_notification_preferences),
        )
        .route(
            "/api/v1/devices/me/preferences",
            put(device_endpoints::update_device_preferences),
        )
        .route(
            "/api/v1/devices/me/subscriptions/{channel_id}",
            put(device_endpoints::update_subscription),
        )
        .route(
            "/api/v1/devices/me/channels/{channel_id}/clear",
            post(device_endpoints::clear_channel),
        )
        .route(
            "/api/v1/requests",
            post(requests::create_request).get(requests::list_requests),
        )
        .route("/api/v1/requests/{request_id}", get(requests::get_request))
        .route(
            "/api/v1/requests/{request_id}/cancel",
            post(requests::cancel_request),
        )
        .route(
            "/api/v1/requests/{request_id}/decision",
            get(requests::get_request_decision),
        )
        .route(
            "/api/v1/requests/{request_id}/wait",
            get(requests::wait_for_request_decision),
        )
        .route(
            "/api/v1/requests/{request_id}/options/{option_id}",
            post(requests::submit_option),
        )
        .route("/api/v1/sync", get(sync_socket::sync_socket))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::nod())
}
