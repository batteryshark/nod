use chrono::{Duration, Utc};
use sqlx::{Row, SqlitePool};

use super::{
    get_user, list_channels_for_device, list_user_devices, now_string,
    validation::{parse_time, validate_id},
};
use crate::{
    auth::{generate_enrollment_code, generate_token, hash_secret, new_id},
    error::ApiError,
    models::{
        CreateEnrollmentCodeRequest, DeviceSigningKeyRequest, EnrollDeviceRequest,
        EnrollDeviceResponse, EnrollmentCodeResponse, NotificationDelivery,
    },
    signing,
};

const MAX_NATIVE_APP_ID_LEN: usize = 255;

pub async fn create_enrollment_code(
    pool: &SqlitePool,
    user_id: &str,
    req: CreateEnrollmentCodeRequest,
) -> Result<EnrollmentCodeResponse, ApiError> {
    validate_id(user_id, "user id")?;
    get_user(pool, user_id).await?;
    let code = generate_enrollment_code();
    let expires_at =
        Utc::now() + Duration::seconds(req.expires_in_seconds.unwrap_or(600).clamp(30, 3600));
    sqlx::query(
        r#"
        INSERT INTO user_enrollment_codes (code_hash, user_id, expires_at, created_at)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(hash_secret(&code))
    .bind(user_id)
    .bind(expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    .bind(now_string())
    .execute(pool)
    .await?;
    Ok(EnrollmentCodeResponse { code, expires_at })
}

pub async fn enroll_device(
    pool: &SqlitePool,
    req: EnrollDeviceRequest,
    notification_delivery: NotificationDelivery,
) -> Result<EnrollDeviceResponse, ApiError> {
    if req.device_name.trim().is_empty() {
        return Err(ApiError::BadRequest("device name is required".to_string()));
    }
    let code_hash = hash_secret(req.code.trim());
    let now = Utc::now();
    let now_text = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let push = normalized_push_registration(&req)?;
    let signing_key = normalized_signing_key(req.signing_key.as_ref())?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        r#"
        SELECT user_id, expires_at, consumed_at
        FROM user_enrollment_codes
        WHERE code_hash = ?
        "#,
    )
    .bind(&code_hash)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;

    let expires_at = parse_time(row.get("expires_at"))?;
    let consumed_at: Option<String> = row.get("consumed_at");
    if consumed_at.is_some() || expires_at <= now {
        return Err(ApiError::Conflict(
            "enrollment code is expired or already used".to_string(),
        ));
    }
    let user_id: String = row.get("user_id");
    let user_name: String =
        sqlx::query_scalar("SELECT name FROM users WHERE id = ? AND deleted_at IS NULL")
            .bind(&user_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(ApiError::NotFound)?;

    let consumed = sqlx::query(
        r#"
        UPDATE user_enrollment_codes
        SET consumed_at = ?
        WHERE code_hash = ? AND consumed_at IS NULL
        "#,
    )
    .bind(&now_text)
    .bind(&code_hash)
    .execute(&mut *transaction)
    .await?;
    if consumed.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "enrollment code is expired or already used".to_string(),
        ));
    }

    let token = generate_token("nod_device");
    let device_id = new_id();
    sqlx::query(
        r#"
        INSERT INTO devices (
            id, user_id, name, platform, native_app_id, token_hash, push_provider, push_token,
            signing_key_id, signing_key_algorithm, signing_public_key, last_seen_at, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&device_id)
    .bind(&user_id)
    .bind(req.device_name.trim())
    .bind(req.platform.as_str())
    .bind(push.native_app_id)
    .bind(hash_secret(&token))
    .bind(push.provider)
    .bind(push.token)
    .bind(signing_key.as_ref().map(|key| key.key_id.as_str()))
    .bind(signing_key.as_ref().map(|key| key.algorithm.as_str()))
    .bind(signing_key.as_ref().map(|key| key.public_key.as_str()))
    .bind(&now_text)
    .bind(&now_text)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;
    let channels = list_channels_for_device(pool, &device_id).await?;
    let devices = list_user_devices(pool, &user_id, &device_id).await?;
    Ok(EnrollDeviceResponse {
        device_id,
        user_id,
        user_name,
        token,
        notification_delivery,
        channels,
        devices,
    })
}

fn normalized_signing_key(
    signing_key: Option<&DeviceSigningKeyRequest>,
) -> Result<Option<DeviceSigningKeyRequest>, ApiError> {
    let Some(signing_key) = signing_key else {
        return Ok(None);
    };
    validate_id(signing_key.key_id.trim(), "signing key id")?;
    if signing_key.algorithm != signing::DEFAULT_ALGORITHM {
        return Err(ApiError::BadRequest(
            "unsupported signing key algorithm".to_string(),
        ));
    }
    let public_key = signing_key.public_key.trim();
    if public_key.is_empty() {
        return Err(ApiError::BadRequest(
            "signing public key is required".to_string(),
        ));
    }
    signing::validate_device_public_key(public_key)?;
    Ok(Some(DeviceSigningKeyRequest {
        key_id: signing_key.key_id.trim().to_string(),
        algorithm: signing_key.algorithm.clone(),
        public_key: public_key.to_string(),
    }))
}

struct PushRegistration {
    native_app_id: Option<String>,
    provider: Option<String>,
    token: Option<String>,
}

fn normalized_push_registration(req: &EnrollDeviceRequest) -> Result<PushRegistration, ApiError> {
    let native_app_id = normalized_native_app_id(req.native_app_id.as_deref())?;
    let provider = req
        .push_provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let token = req
        .push_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (provider, token) {
        (Some(_), Some(_)) if native_app_id.is_none() => Err(ApiError::BadRequest(
            "native app id is required for push registration".to_string(),
        )),
        (Some(provider), Some(token)) => Ok(PushRegistration {
            native_app_id,
            provider: Some(provider.to_string()),
            token: Some(token.to_string()),
        }),
        (None, None) => Ok(PushRegistration {
            native_app_id,
            provider: None,
            token: None,
        }),
        (Some(_), None) => Err(ApiError::BadRequest("push token is required".to_string())),
        (None, Some(_)) => Err(ApiError::BadRequest(
            "push provider is required".to_string(),
        )),
    }
}

pub(super) fn normalized_native_app_id(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > MAX_NATIVE_APP_ID_LEN {
        return Err(ApiError::BadRequest(
            "native app id is too long".to_string(),
        ));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest(
            "native app id must not contain whitespace".to_string(),
        ));
    }
    Ok(Some(value.to_string()))
}
