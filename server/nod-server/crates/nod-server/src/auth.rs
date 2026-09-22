use axum::http::HeaderMap;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use rand::{distributions::Alphanumeric, rngs::OsRng, Rng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    error::ApiError,
    models::{Device, DevicePlatform, IssuerToken},
};

#[derive(Debug, Clone)]
pub enum Principal {
    Admin,
    Device(Box<Device>),
    Issuer(IssuerToken),
}

pub const ADMIN_SESSION_COOKIE: &str = "nod_admin_session";
const ADMIN_SESSION_SECONDS: i64 = 12 * 60 * 60;

pub fn hash_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    hex::encode(digest)
}

pub fn generate_token(prefix: &str) -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn generate_enrollment_code() -> String {
    let mut rng = OsRng;
    (0..8)
        .map(|_| rng.sample(Alphanumeric) as char)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

pub fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    value.strip_prefix("Bearer ").ok_or(ApiError::Unauthorized)
}

pub fn admin_token_matches(token: &str, admin_token: &str) -> bool {
    constant_time_eq(token.as_bytes(), admin_token.as_bytes())
}

pub fn create_admin_session_cookie(admin_token: &str) -> String {
    let expires_at = Utc::now().timestamp() + ADMIN_SESSION_SECONDS;
    let value = admin_session_value(expires_at, admin_token);
    format!(
        "{ADMIN_SESSION_COOKIE}={value}; Path=/; Max-Age={ADMIN_SESSION_SECONDS}; HttpOnly; SameSite=Lax"
    )
}

pub fn expired_admin_session_cookie() -> String {
    format!("{ADMIN_SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
}

pub fn valid_admin_session(headers: &HeaderMap, admin_token: &str) -> bool {
    let Some(value) = cookie_value(headers, ADMIN_SESSION_COOKIE) else {
        return false;
    };
    let Some((expires_at, signature)) = value.split_once('.') else {
        return false;
    };
    let Ok(expires_at) = expires_at.parse::<i64>() else {
        return false;
    };
    if expires_at <= Utc::now().timestamp() {
        return false;
    }

    let expected = admin_session_signature(expires_at, admin_token);
    constant_time_eq(signature.as_bytes(), expected.as_bytes())
}

pub async fn require_admin(headers: &HeaderMap, admin_token: &str) -> Result<(), ApiError> {
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        let token = value
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        if admin_token_matches(token, admin_token) {
            return Ok(());
        }
        return Err(ApiError::Forbidden);
    }

    if valid_admin_session(headers, admin_token) {
        return Ok(());
    }

    Err(ApiError::Unauthorized)
}

pub async fn authenticate(
    headers: &HeaderMap,
    pool: &SqlitePool,
    admin_token: &str,
) -> Result<Principal, ApiError> {
    let token = bearer_token(headers)?;
    if admin_token_matches(token, admin_token) {
        return Ok(Principal::Admin);
    }

    let hash = hash_secret(token);
    if let Some(device) = find_device_by_hash(pool, &hash).await? {
        touch_device(pool, &device.id).await?;
        return Ok(Principal::Device(Box::new(device)));
    }

    if let Some(issuer) = find_issuer_token_by_hash(pool, &hash).await? {
        return Ok(Principal::Issuer(issuer));
    }

    Err(ApiError::Unauthorized)
}

pub async fn require_device(headers: &HeaderMap, pool: &SqlitePool) -> Result<Device, ApiError> {
    let token = bearer_token(headers)?;
    let hash = hash_secret(token);
    let device = find_device_by_hash(pool, &hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    touch_device(pool, &device.id).await?;
    Ok(device)
}

pub fn require_request_write(principal: &Principal, channel_id: &str) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::Issuer(token) if has_request_scope(token, "write", channel_id) => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

pub fn require_request_read(principal: &Principal, channel_id: &str) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::Issuer(token) if has_request_scope(token, "read", channel_id) => Ok(()),
        Principal::Device(_) => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

pub fn require_request_cancel(principal: &Principal, channel_id: &str) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::Issuer(token) if has_request_scope(token, "cancel", channel_id) => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

fn has_request_scope(token: &IssuerToken, operation: &str, channel_id: &str) -> bool {
    let channel_scope = format!("requests:{operation}:{channel_id}");
    let any_channel_scope = format!("requests:*:{channel_id}");
    token.scopes.iter().any(|scope| {
        scope == "*"
            || scope == "requests:*"
            || scope == &format!("requests:{operation}")
            || scope == &channel_scope
            || scope == &any_channel_scope
            || (operation == "read"
                && (scope == "requests:write" || scope == &format!("requests:write:{channel_id}")))
    })
}

pub async fn find_device_by_hash(
    pool: &SqlitePool,
    hash: &str,
) -> Result<Option<Device>, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT id, COALESCE(user_id, 'owner') AS user_id, name, platform, native_app_id,
            push_provider, push_token, signing_key_id, signing_key_algorithm, signing_public_key,
            notification_sound, notification_preferences_json, last_seen_at, created_at
        FROM devices
        WHERE token_hash = ? AND revoked_at IS NULL
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    row.map(|row| {
        Ok(Device {
            id: row.get("id"),
            user_id: row.get("user_id"),
            name: row.get("name"),
            platform: DevicePlatform::from(row.get::<String, _>("platform").as_str()),
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
    })
    .transpose()
}

pub async fn find_issuer_token_by_hash(
    pool: &SqlitePool,
    hash: &str,
) -> Result<Option<IssuerToken>, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT id, name, scopes_json, created_at
        FROM issuer_tokens
        WHERE token_hash = ? AND revoked_at IS NULL
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    row.map(|row| {
        let scopes_json: String = row.get("scopes_json");
        Ok(IssuerToken {
            id: row.get("id"),
            name: row.get("name"),
            scopes: serde_json::from_str(&scopes_json)?,
            created_at: parse_time(row.get("created_at"))?,
        })
    })
    .transpose()
}

async fn touch_device(pool: &SqlitePool, device_id: &str) -> Result<(), ApiError> {
    let cutoff = (Utc::now() - chrono::Duration::seconds(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE devices SET last_seen_at = ? WHERE id = ? AND last_seen_at < ? AND revoked_at IS NULL")
        .bind(crate::db::now_string())
        .bind(device_id)
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

fn parse_time(value: String) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    chrono::DateTime::parse_from_rfc3339(&value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|err| ApiError::Internal(format!("invalid timestamp in database: {err}")))
}

pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

fn admin_session_value(expires_at: i64, admin_token: &str) -> String {
    format!(
        "{expires_at}.{}",
        admin_session_signature(expires_at, admin_token)
    )
}

fn admin_session_signature(expires_at: i64, admin_token: &str) -> String {
    hash_secret(&format!("nod-admin-session:v1:{expires_at}:{admin_token}"))
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(cookie_name, cookie_value)| {
            (cookie_name == name).then(|| cookie_value.to_string())
        })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}
