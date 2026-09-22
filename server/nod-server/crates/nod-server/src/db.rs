use chrono::Utc;
use sqlx::SqlitePool;

mod channels;
mod connection;
mod device_attestations;
mod devices;
mod enrollment;
mod issuer_tokens;
pub(crate) mod push_deliveries;
mod requests;
mod rows;
mod subscriptions;
mod users;
mod validation;

pub use channels::{
    create_channel, delete_channel, get_channel, list_channels, list_channels_for_device,
    list_channels_for_user,
};
pub use device_attestations::record_device_attestation;
pub use devices::{
    delete_device, get_device, list_devices_for_admin, list_user_devices, rename_user_device,
    revoke_user_device,
};
pub use enrollment::{create_enrollment_code, enroll_device};
pub use issuer_tokens::{create_issuer_token, list_issuer_tokens_for_admin, revoke_issuer_token};
pub use subscriptions::{
    clear_channel, set_subscription, set_user_subscription, update_device_preferences,
    update_notification_preferences, update_push_token,
};
pub use users::{
    create_user, delete_user, get_user, list_user_subscriptions_for_admin, list_users_for_admin,
    update_user, update_user_subscriptions_for_admin,
};

use crate::{error::ApiError, models::AdminCounts};
pub use connection::connect;
pub use requests::{
    cancel_request, create_request, expire_due_requests, get_request, list_requests_for_device,
    prune_retention, recent_requests, record_decision, request_created_by_issuer_token_id,
    request_delivery_eligible, request_for_user, request_visible_to_user, CreateRequestMetadata,
    DecisionSubmission, ListRequestsForDevice,
};

pub const DEFAULT_USER_ID: &str = "owner";

pub fn now_string() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub async fn admin_counts(pool: &SqlitePool) -> Result<AdminCounts, ApiError> {
    let users = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await?;
    let channels = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(pool)
        .await?;
    let devices = sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL")
        .fetch_one(pool)
        .await?;
    let active_issuer_tokens =
        sqlx::query_scalar("SELECT COUNT(*) FROM issuer_tokens WHERE revoked_at IS NULL")
            .fetch_one(pool)
            .await?;
    let pending_requests =
        sqlx::query_scalar("SELECT COUNT(*) FROM requests WHERE status = 'pending'")
            .fetch_one(pool)
            .await?;
    Ok(AdminCounts {
        users,
        channels,
        devices,
        active_issuer_tokens,
        pending_requests,
    })
}
