use sqlx::{Row, SqlitePool};

use super::{
    list_channels_for_user, now_string,
    rows::row_to_user,
    set_user_subscription,
    validation::{parse_time, validate_id},
};
use crate::{
    error::ApiError,
    models::{
        AdminUser, AdminUserSubscriptionUpdate, Channel, CreateUserRequest, UpdateUserRequest, User,
    },
};

pub async fn list_users_for_admin(pool: &SqlitePool) -> Result<Vec<AdminUser>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT
            u.id,
            u.name,
            u.created_at,
            u.updated_at,
            (
                SELECT COUNT(*)
                FROM devices d
                WHERE d.user_id = u.id AND d.revoked_at IS NULL
            ) AS device_count,
            (
                SELECT COUNT(*)
                FROM user_channel_subscriptions us
                WHERE us.user_id = u.id
                    AND us.subscribed = 1
            ) AS subscribed_channel_count,
            (
                SELECT COALESCE(group_concat(us.channel_id, char(31)), '')
                FROM user_channel_subscriptions us
                WHERE us.user_id = u.id
                    AND us.subscribed = 1
            ) AS subscribed_channel_ids
        FROM users u
        WHERE u.deleted_at IS NULL
        ORDER BY u.name
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let subscribed_channel_ids = row.get::<String, _>("subscribed_channel_ids");
            Ok(AdminUser {
                id: row.get("id"),
                name: row.get("name"),
                device_count: row.get("device_count"),
                subscribed_channel_count: row.get("subscribed_channel_count"),
                subscribed_channel_ids: subscribed_channel_ids
                    .split('\x1f')
                    .filter(|channel_id| !channel_id.is_empty())
                    .map(str::to_string)
                    .collect(),
                created_at: parse_time(row.get("created_at"))?,
                updated_at: parse_time(row.get("updated_at"))?,
            })
        })
        .collect()
}

pub async fn create_user(pool: &SqlitePool, req: CreateUserRequest) -> Result<User, ApiError> {
    validate_id(&req.id, "user id")?;
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("user name is required".to_string()));
    }
    let now = now_string();
    let inserted = sqlx::query(
        r#"
        INSERT INTO users (id, name, created_at, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            updated_at = excluded.updated_at
        WHERE users.deleted_at IS NULL
        "#,
    )
    .bind(&req.id)
    .bind(req.name.trim())
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "deleted user IDs cannot be reused".to_string(),
        ));
    }

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO user_channel_subscriptions (user_id, channel_id, subscribed, updated_at)
        VALUES (?, 'default', 1, ?)
        "#,
    )
    .bind(&req.id)
    .bind(now_string())
    .execute(pool)
    .await?;

    get_user(pool, &req.id).await
}

pub async fn update_user(
    pool: &SqlitePool,
    user_id: &str,
    req: UpdateUserRequest,
) -> Result<User, ApiError> {
    validate_id(user_id, "user id")?;
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("user name is required".to_string()));
    }
    let updated = sqlx::query(
        "UPDATE users SET name = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(req.name.trim())
    .bind(now_string())
    .bind(user_id)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    get_user(pool, user_id).await
}

pub async fn get_user(pool: &SqlitePool, user_id: &str) -> Result<User, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT id, name, created_at, updated_at
        FROM users
        WHERE id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row_to_user(row)
}

pub async fn delete_user(pool: &SqlitePool, user_id: &str) -> Result<(), ApiError> {
    validate_id(user_id, "user id")?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let device_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE user_id = ? AND revoked_at IS NULL")
            .bind(user_id)
            .fetch_one(&mut *transaction)
            .await?;
    if device_count > 0 {
        return Err(ApiError::Conflict(
            "cannot delete a user that still owns devices".to_string(),
        ));
    }
    let deleted =
        sqlx::query("UPDATE users SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
            .bind(now_string())
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn list_user_subscriptions_for_admin(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<Channel>, ApiError> {
    get_user(pool, user_id).await?;
    list_channels_for_user(pool, user_id).await
}

pub async fn update_user_subscriptions_for_admin(
    pool: &SqlitePool,
    user_id: &str,
    updates: &[AdminUserSubscriptionUpdate],
) -> Result<Vec<Channel>, ApiError> {
    get_user(pool, user_id).await?;
    for update in updates {
        set_user_subscription(pool, user_id, &update.channel_id, update.subscribed).await?;
    }
    list_channels_for_user(pool, user_id).await
}
