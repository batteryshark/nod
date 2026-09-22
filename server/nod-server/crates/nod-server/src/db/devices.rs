use sqlx::{Row, SqlitePool};

use super::{
    get_user, list_channels_for_device, now_string,
    rows::{row_to_attestation_summary, row_to_device, row_to_user_device},
    validation::parse_time,
    DEFAULT_USER_ID,
};
use crate::{
    error::ApiError,
    models::{AdminDevice, Device, UpdateUserDeviceRequest, UserDevice},
};

pub async fn list_devices_for_admin(pool: &SqlitePool) -> Result<Vec<AdminDevice>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT
            d.id,
            COALESCE(d.user_id, ?) AS user_id,
            COALESCE(u.name, 'Owner') AS user_name,
            d.name,
            d.platform,
            d.native_app_id,
            d.push_provider,
            d.push_token,
            d.signing_key_id,
            d.signing_key_algorithm,
            d.signing_public_key,
            d.notification_sound, d.notification_preferences_json,
            da.provider AS attestation_provider,
            da.status AS attestation_status,
            da.key_id AS attestation_key_id,
            da.team_id AS attestation_team_id,
            da.bundle_id AS attestation_bundle_id,
            da.environment AS attestation_environment,
            da.verified_at AS attestation_verified_at,
            da.failure_reason AS attestation_failure_reason,
            d.last_seen_at,
            d.created_at
        FROM devices d
        LEFT JOIN users u ON u.id = d.user_id
        LEFT JOIN device_attestations da
            ON da.device_id = d.id AND da.provider = 'apple_app_attest'
        WHERE d.revoked_at IS NULL
        ORDER BY d.created_at DESC
        "#,
    )
    .bind(DEFAULT_USER_ID)
    .fetch_all(pool)
    .await?;

    let mut devices = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let subscriptions = list_channels_for_device(pool, &id).await?;
        devices.push(AdminDevice {
            id,
            user_id: row.get("user_id"),
            user_name: row.get("user_name"),
            name: row.get("name"),
            platform: crate::models::DevicePlatform::from(
                row.get::<String, _>("platform").as_str(),
            ),
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
            subscriptions,
        });
    }
    Ok(devices)
}

pub async fn get_device(pool: &SqlitePool, device_id: &str) -> Result<Device, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT id, COALESCE(user_id, ?) AS user_id, name, platform, native_app_id,
            push_provider, push_token, signing_key_id, signing_key_algorithm, signing_public_key,
            notification_sound, notification_preferences_json, last_seen_at, created_at
        FROM devices
        WHERE id = ? AND revoked_at IS NULL
        "#,
    )
    .bind(DEFAULT_USER_ID)
    .bind(device_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row_to_device(row)
}

pub async fn delete_device(pool: &SqlitePool, device_id: &str) -> Result<(), ApiError> {
    let decision = sqlx::query(
        "UPDATE devices SET revoked_at = ?, push_token = NULL WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(now_string())
    .bind(device_id)
    .execute(pool)
    .await?;
    if decision.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn list_user_devices(
    pool: &SqlitePool,
    user_id: &str,
    current_device_id: &str,
) -> Result<Vec<UserDevice>, ApiError> {
    get_user(pool, user_id).await?;
    let rows = sqlx::query(
        r#"
        SELECT d.id, COALESCE(d.user_id, ?) AS user_id, d.name, d.platform,
            d.native_app_id, d.push_provider, d.push_token, d.signing_key_id, d.signing_key_algorithm,
            d.signing_public_key, d.notification_sound, d.notification_preferences_json,
            da.provider AS attestation_provider,
            da.status AS attestation_status,
            da.key_id AS attestation_key_id,
            da.team_id AS attestation_team_id,
            da.bundle_id AS attestation_bundle_id,
            da.environment AS attestation_environment,
            da.verified_at AS attestation_verified_at,
            da.failure_reason AS attestation_failure_reason,
            d.last_seen_at, d.created_at
        FROM devices d
        LEFT JOIN device_attestations da
            ON da.device_id = d.id AND da.provider = 'apple_app_attest'
        WHERE d.revoked_at IS NULL AND COALESCE(d.user_id, ?) = ?
        ORDER BY d.created_at DESC
        "#,
    )
    .bind(DEFAULT_USER_ID)
    .bind(DEFAULT_USER_ID)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| row_to_user_device(row, current_device_id))
        .collect()
}

pub async fn rename_user_device(
    pool: &SqlitePool,
    user_id: &str,
    device_id: &str,
    current_device_id: &str,
    req: UpdateUserDeviceRequest,
) -> Result<UserDevice, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("device name is required".to_string()));
    }
    let updated = sqlx::query(
        r#"
        UPDATE devices
        SET name = ?, last_seen_at = ?
        WHERE id = ? AND revoked_at IS NULL AND COALESCE(user_id, ?) = ?
        "#,
    )
    .bind(req.name.trim())
    .bind(now_string())
    .bind(device_id)
    .bind(DEFAULT_USER_ID)
    .bind(user_id)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    get_user_device(pool, user_id, device_id, current_device_id).await
}

pub async fn revoke_user_device(
    pool: &SqlitePool,
    user_id: &str,
    device_id: &str,
) -> Result<(), ApiError> {
    let deleted = sqlx::query(
        r#"
        UPDATE devices SET revoked_at = ?, push_token = NULL
        WHERE id = ? AND revoked_at IS NULL AND COALESCE(user_id, ?) = ?
        "#,
    )
    .bind(now_string())
    .bind(device_id)
    .bind(DEFAULT_USER_ID)
    .bind(user_id)
    .execute(pool)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

async fn get_user_device(
    pool: &SqlitePool,
    user_id: &str,
    device_id: &str,
    current_device_id: &str,
) -> Result<UserDevice, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT d.id, COALESCE(d.user_id, ?) AS user_id, d.name, d.platform,
            d.native_app_id, d.push_provider, d.push_token, d.signing_key_id, d.signing_key_algorithm,
            d.signing_public_key, d.notification_sound, d.notification_preferences_json,
            da.provider AS attestation_provider,
            da.status AS attestation_status,
            da.key_id AS attestation_key_id,
            da.team_id AS attestation_team_id,
            da.bundle_id AS attestation_bundle_id,
            da.environment AS attestation_environment,
            da.verified_at AS attestation_verified_at,
            da.failure_reason AS attestation_failure_reason,
            d.last_seen_at, d.created_at
        FROM devices d
        LEFT JOIN device_attestations da
            ON da.device_id = d.id AND da.provider = 'apple_app_attest'
        WHERE d.id = ? AND d.revoked_at IS NULL AND COALESCE(d.user_id, ?) = ?
        "#,
    )
    .bind(DEFAULT_USER_ID)
    .bind(device_id)
    .bind(DEFAULT_USER_ID)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row_to_user_device(row, current_device_id)
}
