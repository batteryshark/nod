use sqlx::SqlitePool;

use super::{
    enrollment::normalized_native_app_id, get_channel, get_device, get_user, now_string,
    validation::normalize_notification_sound,
};
use crate::{error::ApiError, models::UpdatePushTokenRequest};

pub async fn set_subscription(
    pool: &SqlitePool,
    device_id: &str,
    channel_id: &str,
    subscribed: bool,
) -> Result<(), ApiError> {
    let device = get_device(pool, device_id).await?;
    set_user_subscription(pool, &device.user_id, channel_id, subscribed).await
}

pub async fn set_user_subscription(
    pool: &SqlitePool,
    user_id: &str,
    channel_id: &str,
    subscribed: bool,
) -> Result<(), ApiError> {
    get_user(pool, user_id).await?;
    get_channel(pool, channel_id).await?;
    sqlx::query(
        r#"
        INSERT INTO user_channel_subscriptions (user_id, channel_id, subscribed, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(user_id, channel_id) DO UPDATE SET
            subscribed = excluded.subscribed,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(user_id)
    .bind(channel_id)
    .bind(if subscribed { 1 } else { 0 })
    .bind(now_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_channel(
    pool: &SqlitePool,
    device_id: &str,
    channel_id: &str,
) -> Result<(), ApiError> {
    let device = get_device(pool, device_id).await?;
    get_channel(pool, channel_id).await?;
    sqlx::query(
        r#"
        INSERT INTO user_channel_clears (user_id, channel_id, cleared_at)
        VALUES (?, ?, ?)
        ON CONFLICT(user_id, channel_id) DO UPDATE SET
            cleared_at = excluded.cleared_at
        "#,
    )
    .bind(&device.user_id)
    .bind(channel_id)
    .bind(now_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_push_token(
    pool: &SqlitePool,
    device_id: &str,
    req: UpdatePushTokenRequest,
) -> Result<(), ApiError> {
    let provider = req.provider.trim();
    let token = req.token.trim();
    if provider.is_empty() {
        return Err(ApiError::BadRequest(
            "push provider is required".to_string(),
        ));
    }
    if token.is_empty() {
        return Err(ApiError::BadRequest("push token is required".to_string()));
    }
    let native_app_id = normalized_native_app_id(Some(&req.native_app_id))?
        .ok_or_else(|| ApiError::BadRequest("native app id is required".to_string()))?;
    sqlx::query(
        r#"
        UPDATE devices
        SET push_provider = ?, push_token = ?, native_app_id = ?, last_seen_at = ?
        WHERE id = ? AND revoked_at IS NULL
        "#,
    )
    .bind(provider)
    .bind(token)
    .bind(native_app_id)
    .bind(now_string())
    .bind(device_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_device_preferences(
    pool: &SqlitePool,
    device_id: &str,
    req: crate::models::UpdateDevicePreferencesRequest,
) -> Result<(), ApiError> {
    if let Some(notification_sound) = req.notification_sound {
        let notification_sound = normalize_notification_sound(&notification_sound)?;
        sqlx::query("UPDATE devices SET notification_sound = ?, last_seen_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(notification_sound)
            .bind(now_string())
            .bind(device_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn update_notification_preferences(
    pool: &SqlitePool,
    device_id: &str,
    mut preferences: nod_proto::DeviceNotificationPreferences,
) -> Result<nod_proto::DeviceNotificationPreferences, ApiError> {
    if preferences.muted_channels.len() > 100 {
        return Err(ApiError::BadRequest(
            "at most 100 muted channels are supported".to_string(),
        ));
    }
    for channel_id in &mut preferences.muted_channels {
        *channel_id = channel_id.trim().to_string();
        super::validation::validate_id(channel_id, "muted channel id")?;
    }
    preferences.muted_channels.sort();
    preferences.muted_channels.dedup();
    let updated=sqlx::query("UPDATE devices SET notification_preferences_json=?,last_seen_at=? WHERE id=? AND revoked_at IS NULL")
        .bind(serde_json::to_string(&preferences)?).bind(now_string()).bind(device_id).execute(pool).await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(preferences)
}
