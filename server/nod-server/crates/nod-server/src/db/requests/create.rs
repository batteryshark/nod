use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::read::get_request_on;
use crate::{
    auth::new_id,
    db::{
        now_string,
        validation::{normalize_options, validate_id, validate_request},
    },
    error::ApiError,
    models::{
        CreateDecisionRequest, CreatedDecisionRequest, DecisionRequest, DecisionResolution,
        RequestOption,
    },
};

#[derive(Default)]
pub struct CreateRequestMetadata<'a> {
    pub created_by_issuer_token_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

pub async fn create_request(
    pool: &SqlitePool,
    req: CreateDecisionRequest,
    metadata: CreateRequestMetadata<'_>,
) -> Result<CreatedDecisionRequest, ApiError> {
    validate_request(&req)?;
    validate_options(&req.options)?;
    let idempotency_key = metadata.idempotency_key.map(str::trim);
    if idempotency_key.is_some_and(|key| key.is_empty() || key.len() > 128) {
        return Err(ApiError::BadRequest(
            "idempotency_key must contain 1 to 128 bytes".to_string(),
        ));
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&req)?));
    let scope = metadata.created_by_issuer_token_id.unwrap_or("admin");
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(key) = idempotency_key {
        if let Some(row) = sqlx::query("SELECT request_id,fingerprint FROM request_idempotency WHERE scope=? AND channel_id=? AND key=?")
            .bind(scope).bind(&req.channel_id).bind(key).fetch_optional(&mut *transaction).await? {
            if row.get::<String,_>("fingerprint") != fingerprint {
                return Err(ApiError::Conflict("idempotency_key was already used for different request content".to_string()));
            }
            let request = get_request_on(&mut transaction, &row.get::<String,_>("request_id")).await?;
            transaction.commit().await?;
            return Ok(CreatedDecisionRequest { request_id: request.id.clone(), deduped: true, request });
        }
    }
    let channel_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id = ?)")
            .bind(&req.channel_id)
            .fetch_one(&mut *transaction)
            .await?;
    if !channel_exists {
        return Err(ApiError::NotFound);
    }
    let recipients =
        resolve_request_recipients(&mut transaction, &req.channel_id, req.recipients.as_ref())
            .await?;
    // The dedupe key is part of the issuer contract: retried creates must return the pending request.
    if let Some(key) = req.dedupe_key.as_deref() {
        if let Some(request) =
            find_pending_request_by_dedupe_key(&mut transaction, &req.channel_id, key).await?
        {
            bind_idempotency(
                &mut transaction,
                IdempotencyBinding {
                    scope,
                    channel_id: &req.channel_id,
                    key: idempotency_key,
                    fingerprint: &fingerprint,
                    request_id: &request.id,
                },
            )
            .await?;
            transaction.commit().await?;
            return Ok(CreatedDecisionRequest {
                request_id: request.id.clone(),
                deduped: true,
                request,
            });
        }
    }

    let now = now_string();
    let id = new_id();
    let decision_resolution = req
        .decision_resolution
        .unwrap_or(DecisionResolution::Shared);
    let summary = if req.summary.trim().is_empty() {
        req.body_markdown
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(160)
            .collect()
    } else {
        req.summary
    };
    let notification = normalized_notification(req.notification);
    let options = normalize_options(req.options);

    sqlx::query(
        r#"
        INSERT INTO requests (
            id, channel_id, title, summary, body_markdown, fields_json, links_json,
            image_url, notification_json, dedupe_key, expires_at, status,
            created_at, updated_at, callback_url, decision_resolution, created_by_issuer_token_id, recipient_salt, explicit_recipients
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, ?, lower(hex(randomblob(32))), ?)
        "#,
    )
    .bind(&id)
    .bind(&req.channel_id)
    .bind(req.title.trim())
    .bind(summary.trim())
    .bind(req.body_markdown.trim())
    .bind(serde_json::to_string(&req.fields)?)
    .bind(serde_json::to_string(&req.links)?)
    .bind(req.image_url)
    .bind(serde_json::to_string(&notification)?)
    .bind(req.dedupe_key)
    .bind(
        req.expires_at
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    )
    .bind(&now)
    .bind(&now)
    .bind(req.callback_url)
    .bind(decision_resolution.as_str())
    .bind(metadata.created_by_issuer_token_id)
    .bind(req.recipients.is_some())
    .execute(&mut *transaction)
    .await?;

    for user_id in &recipients {
        insert_request_recipient(&mut transaction, &id, user_id).await?;
    }

    for option in options {
        insert_option(&mut transaction, &id, option).await?;
    }

    // Keep the push outbox atomic with creation so a process restart between
    // committing the request and broadcasting it cannot lose push delivery.
    sqlx::query("INSERT INTO push_deliveries(request_id,device_id,status,attempts,updated_at) SELECT ?,d.id,'queued',0,? FROM devices d JOIN request_recipients r ON r.user_id=d.user_id AND r.request_id=? WHERE d.revoked_at IS NULL AND d.push_token IS NOT NULL AND TRIM(d.push_token)!='' AND d.native_app_id IS NOT NULL AND TRIM(d.native_app_id)!=''")
        .bind(&id).bind(&now).bind(&id).execute(&mut *transaction).await?;

    bind_idempotency(
        &mut transaction,
        IdempotencyBinding {
            scope,
            channel_id: &req.channel_id,
            key: idempotency_key,
            fingerprint: &fingerprint,
            request_id: &id,
        },
    )
    .await?;
    let request = get_request_on(&mut transaction, &id).await?;
    transaction.commit().await?;
    Ok(CreatedDecisionRequest {
        request_id: id,
        deduped: false,
        request,
    })
}

fn normalized_notification(
    mut notification: crate::models::RequestNotification,
) -> crate::models::RequestNotification {
    notification.title = normalized_optional_text(notification.title);
    notification.body = normalized_optional_text(notification.body);
    notification
}

fn normalized_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn insert_option(
    connection: &mut SqliteConnection,
    request_id: &str,
    option: RequestOption,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO request_options (
            request_id, option_id, kind, label, style, requires_text, text_placeholder,
            destructive, foreground, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(request_id)
    .bind(option.id)
    .bind(option.kind.as_str())
    .bind(option.label)
    .bind(option.style)
    .bind(if option.requires_text { 1 } else { 0 })
    .bind(option.text_placeholder)
    .bind(if option.destructive { 1 } else { 0 })
    .bind(if option.foreground { 1 } else { 0 })
    .bind(now_string())
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn insert_request_recipient(
    connection: &mut SqliteConnection,
    request_id: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO request_recipients (request_id, user_id, created_at)
        VALUES (?, ?, ?)
        "#,
    )
    .bind(request_id)
    .bind(user_id)
    .bind(now_string())
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn resolve_request_recipients(
    connection: &mut SqliteConnection,
    channel_id: &str,
    requested: Option<&Vec<String>>,
) -> Result<Vec<String>, ApiError> {
    if let Some(requested) = requested {
        if requested.is_empty() {
            return Err(ApiError::BadRequest(
                "recipients must not be empty when provided".to_string(),
            ));
        }

        let mut recipients = Vec::with_capacity(requested.len());
        for user_id in requested {
            let user_id = user_id.trim();
            validate_id(user_id, "user id")?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id = ? AND deleted_at IS NULL)",
            )
            .bind(user_id)
            .fetch_one(&mut *connection)
            .await?;
            if !exists {
                return Err(ApiError::NotFound);
            }
            if !recipients.iter().any(|existing| existing == user_id) {
                recipients.push(user_id.to_string());
            }
        }
        return Ok(recipients);
    }

    let rows = sqlx::query(
        r#"
        SELECT user_id
        FROM user_channel_subscriptions
        JOIN users ON users.id = user_id AND users.deleted_at IS NULL
        WHERE channel_id = ? AND subscribed = 1
        ORDER BY user_id
        "#,
    )
    .bind(channel_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(rows.into_iter().map(|row| row.get("user_id")).collect())
}

async fn find_pending_request_by_dedupe_key(
    connection: &mut SqliteConnection,
    channel_id: &str,
    dedupe_key: &str,
) -> Result<Option<DecisionRequest>, ApiError> {
    let row = sqlx::query(
        "SELECT id FROM requests WHERE channel_id = ? AND dedupe_key = ? AND status = 'pending'",
    )
    .bind(channel_id)
    .bind(dedupe_key)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = row {
        Ok(Some(
            get_request_on(connection, row.get::<String, _>("id").as_str()).await?,
        ))
    } else {
        Ok(None)
    }
}

fn validate_options(options: &[RequestOption]) -> Result<(), ApiError> {
    let mut ids = std::collections::HashSet::new();
    for option in options {
        validate_id(&option.id, "option id")?;
        if option.label.trim().is_empty() {
            return Err(ApiError::BadRequest("option label is required".to_string()));
        }
        if !ids.insert(&option.id) {
            return Err(ApiError::BadRequest(
                "option IDs must be unique".to_string(),
            ));
        }
    }
    Ok(())
}

struct IdempotencyBinding<'a> {
    scope: &'a str,
    channel_id: &'a str,
    key: Option<&'a str>,
    fingerprint: &'a str,
    request_id: &'a str,
}

async fn bind_idempotency(
    connection: &mut SqliteConnection,
    binding: IdempotencyBinding<'_>,
) -> Result<(), ApiError> {
    if let Some(key) = binding.key {
        sqlx::query("INSERT INTO request_idempotency(scope,channel_id,key,fingerprint,request_id) VALUES(?,?,?,?,?)")
            .bind(binding.scope).bind(binding.channel_id).bind(key).bind(binding.fingerprint).bind(binding.request_id).execute(connection).await?;
    }
    Ok(())
}
