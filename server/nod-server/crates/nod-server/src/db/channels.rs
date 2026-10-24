use sqlx::SqlitePool;
use unicode_segmentation::UnicodeSegmentation;

use super::{get_device, now_string, rows::row_to_channel, validation::validate_id};
use crate::{
    error::ApiError,
    models::{Channel, CreateChannelRequest},
};

pub async fn list_channels(pool: &SqlitePool) -> Result<Vec<Channel>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, name, emoji, 1 AS subscribed, created_at FROM channels ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_channel).collect()
}

pub async fn list_channels_for_device(
    pool: &SqlitePool,
    device_id: &str,
) -> Result<Vec<Channel>, ApiError> {
    let device = get_device(pool, device_id).await?;
    list_channels_for_user(pool, &device.user_id).await
}

pub async fn list_channels_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<Channel>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT
            c.id,
            c.name,
            c.emoji,
            COALESCE(us.subscribed, 0) AS subscribed,
            c.created_at
        FROM channels c
        LEFT JOIN user_channel_subscriptions us
            ON us.channel_id = c.id
            AND us.user_id = ?
        ORDER BY c.name
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_channel).collect()
}

pub async fn create_channel(
    pool: &SqlitePool,
    req: CreateChannelRequest,
) -> Result<Channel, ApiError> {
    validate_id(&req.id, "channel id")?;
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("channel name is required".to_string()));
    }
    let now = now_string();
    sqlx::query(
        r#"
        INSERT INTO channels (id, name, emoji, created_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            emoji = excluded.emoji
        "#,
    )
    .bind(&req.id)
    .bind(req.name.trim())
    .bind(normalized_emoji(&req.emoji))
    .bind(now)
    .execute(pool)
    .await?;

    get_channel(pool, &req.id).await
}

pub async fn get_channel(pool: &SqlitePool, channel_id: &str) -> Result<Channel, ApiError> {
    let row = sqlx::query(
        "SELECT id, name, emoji, 1 AS subscribed, created_at FROM channels WHERE id = ?",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row_to_channel(row)
}

fn normalized_emoji(value: &str) -> String {
    let emoji = value.trim();
    if emoji.is_empty() {
        "🔔".to_string()
    } else {
        // Preserve the legacy eight-unit allowance, but never cut a flag,
        // modifier or ZWJ family in the middle of an extended grapheme.
        emoji.graphemes(true).take(8).collect()
    }
}

pub async fn delete_channel(pool: &SqlitePool, channel_id: &str) -> Result<(), ApiError> {
    validate_id(channel_id, "channel id")?;
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        DELETE FROM request_user_decisions
        WHERE request_id IN (SELECT id FROM requests WHERE channel_id = ?)
        "#,
    )
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        DELETE FROM request_recipients
        WHERE request_id IN (SELECT id FROM requests WHERE channel_id = ?)
        "#,
    )
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        DELETE FROM request_options
        WHERE request_id IN (SELECT id FROM requests WHERE channel_id = ?)
        "#,
    )
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM requests WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM user_channel_clears WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM user_channel_subscriptions WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    let deleted = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod emoji_tests {
    use super::normalized_emoji;

    #[test]
    fn preserves_short_legacy_values_and_complete_emoji_clusters() {
        for value in ["abcdefgh", "👨‍👩‍👧‍👦", "👩🏽‍❤️‍💋‍👨🏻", "🇺🇸", "🏴‍☠️", "👍🏿"]
        {
            assert_eq!(normalized_emoji(value), value);
        }
        assert_eq!(normalized_emoji("  "), "🔔");
        assert_eq!(normalized_emoji("1234567👩🏽‍❤️‍💋‍👨🏻9"), "1234567👩🏽‍❤️‍💋‍👨🏻");
    }
}
