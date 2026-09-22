use sqlx::Row;

use super::validation::parse_time;
use crate::{
    error::ApiError,
    models::{
        Channel, Device, DeviceAttestationStatus, DeviceAttestationSummary, User, UserDevice,
    },
};

pub(super) fn row_to_channel(row: sqlx::sqlite::SqliteRow) -> Result<Channel, ApiError> {
    Ok(Channel {
        id: row.get("id"),
        name: row.get("name"),
        emoji: row.get("emoji"),
        subscribed: row.get::<i64, _>("subscribed") != 0,
        created_at: parse_time(row.get("created_at"))?,
    })
}

pub(super) fn row_to_device(row: sqlx::sqlite::SqliteRow) -> Result<Device, ApiError> {
    Ok(Device {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        platform: crate::models::DevicePlatform::from(row.get::<String, _>("platform").as_str()),
        native_app_id: row.get("native_app_id"),
        push_provider: row.get("push_provider"),
        push_token: row.get("push_token"),
        signing_key_id: row.get("signing_key_id"),
        signing_key_algorithm: row.get("signing_key_algorithm"),
        signing_public_key: row.get("signing_public_key"),
        notification_sound: row.get("notification_sound"),
        notification_preferences: serde_json::from_str(
            &row.get::<String, _>("notification_preferences_json"),
        )?,
        last_seen_at: parse_time(row.get("last_seen_at"))?,
        created_at: parse_time(row.get("created_at"))?,
    })
}

pub(super) fn row_to_user_device(
    row: sqlx::sqlite::SqliteRow,
    current_device_id: &str,
) -> Result<UserDevice, ApiError> {
    let id: String = row.get("id");
    Ok(UserDevice {
        is_current: id == current_device_id,
        id,
        user_id: row.get("user_id"),
        name: row.get("name"),
        platform: crate::models::DevicePlatform::from(row.get::<String, _>("platform").as_str()),
        native_app_id: row.get("native_app_id"),
        push_provider: row.get("push_provider"),
        has_push_token: row.get::<Option<String>, _>("push_token").is_some(),
        has_signing_key: row.get::<Option<String>, _>("signing_public_key").is_some(),
        notification_sound: row.get("notification_sound"),
        notification_preferences: serde_json::from_str(
            &row.get::<String, _>("notification_preferences_json"),
        )?,
        attestation: row_to_attestation_summary(&row)?,
        last_seen_at: parse_time(row.get("last_seen_at"))?,
        created_at: parse_time(row.get("created_at"))?,
    })
}

pub(super) fn row_to_attestation_summary(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Option<DeviceAttestationSummary>, ApiError> {
    let provider: Option<String> = row.get("attestation_provider");
    let Some(provider) = provider else {
        return Ok(None);
    };
    let verified_at = row
        .get::<Option<String>, _>("attestation_verified_at")
        .map(parse_time)
        .transpose()?;
    Ok(Some(DeviceAttestationSummary {
        provider,
        status: DeviceAttestationStatus::from(row.get::<String, _>("attestation_status").as_str()),
        key_id: row.get("attestation_key_id"),
        team_id: row.get("attestation_team_id"),
        bundle_id: row.get("attestation_bundle_id"),
        environment: row.get("attestation_environment"),
        verified_at,
        failure_reason: row.get("attestation_failure_reason"),
    }))
}

pub(super) fn row_to_user(row: sqlx::sqlite::SqliteRow) -> Result<User, ApiError> {
    Ok(User {
        id: row.get("id"),
        name: row.get("name"),
        created_at: parse_time(row.get("created_at"))?,
        updated_at: parse_time(row.get("updated_at"))?,
    })
}
