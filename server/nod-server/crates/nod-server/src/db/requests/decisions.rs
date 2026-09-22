use chrono::Utc;
use sqlx::{SqliteConnection, SqlitePool};

use super::read::get_request_on;
use crate::{
    db::{now_string, validation::implicit_dismiss_option},
    error::ApiError,
    models::{
        Decision, DecisionRequest, DecisionResolution, DecisionSignatureRecord, Device,
        RequestOption, RequestStatus, SubmitDecisionRequest,
    },
    signing,
};

pub struct DecisionSubmission<'a> {
    pub request_id: &'a str,
    pub option_id: &'a str,
    pub actor_device: Option<&'a Device>,
    pub actor_user_id: Option<&'a str>,
    pub decision: SubmitDecisionRequest,
}

pub async fn record_decision(
    pool: &SqlitePool,
    submission: DecisionSubmission<'_>,
) -> Result<DecisionRequest, ApiError> {
    let request_id = submission.request_id;
    let option_id = submission.option_id;
    let actor_device = submission.actor_device;
    let actor_user_id = submission.actor_user_id;
    let submitted_decision = submission.decision;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let request = get_request_on(&mut transaction, request_id).await?;
    if let Some(device) = actor_device {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id = ? AND revoked_at IS NULL)",
        )
        .bind(&device.id)
        .fetch_one(&mut *transaction)
        .await?;
        if !active {
            return Err(ApiError::Unauthorized);
        }
    }
    if let Some(user_id) = actor_user_id {
        if !request
            .recipients
            .iter()
            .any(|recipient| recipient == user_id)
        {
            return Err(ApiError::Forbidden);
        }
    }
    if let Some(expires_at) = request.expires_at {
        if expires_at <= Utc::now() {
            // The expiry sweep owns the transition and its broadcast. A
            // rejected decision must not consume that pending transition.
            return Err(ApiError::Conflict("request has expired".to_string()));
        }
    }
    let option = request
        .options
        .iter()
        .find(|option| option.id == option_id)
        .cloned()
        .or_else(|| implicit_dismiss_option(&request, option_id))
        .ok_or(ApiError::NotFound)?;
    if !matches!(request.status, RequestStatus::Pending) {
        return Err(ApiError::Conflict(
            "request is no longer pending".to_string(),
        ));
    }

    let resolved_at = Utc::now();
    let text = submitted_decision
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned);
    let signature = verified_decision_signature(
        &mut transaction,
        &request,
        &option,
        actor_device,
        text.as_deref(),
        submitted_decision.signature.as_ref(),
    )
    .await?;
    let decision = Decision {
        request_id: request_id.to_string(),
        option_id: option.id.clone(),
        option_kind: option.kind.clone(),
        option_label: option.label.clone(),
        text,
        actor_user_id: actor_user_id.map(ToOwned::to_owned),
        actor_device_id: actor_device.map(|device| device.id.clone()),
        signature,
        resolved_at,
    };
    if request.decision_resolution == DecisionResolution::PerUser {
        let Some(user_id) = actor_user_id else {
            return Err(ApiError::BadRequest(
                "per-user options require a user actor".to_string(),
            ));
        };

        // Each recipient resolves their own projection; the shared request closes after the last decision.
        let decision_json = serde_json::to_string(&decision)?;
        let resolved_at_text = resolved_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let inserted = sqlx::query(
            r#"
            INSERT OR IGNORE INTO request_user_decisions (request_id, user_id, decision_json, resolved_at)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(request_id)
        .bind(user_id)
        .bind(decision_json)
        .bind(&resolved_at_text)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() == 0 {
            return Err(ApiError::Conflict(
                "request was already handled by this user".to_string(),
            ));
        }

        let recipient_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM request_recipients WHERE request_id = ?")
                .bind(request_id)
                .fetch_one(&mut *transaction)
                .await?;
        let decision_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM request_user_decisions WHERE request_id = ?")
                .bind(request_id)
                .fetch_one(&mut *transaction)
                .await?;
        if recipient_count > 0 && decision_count >= recipient_count {
            sqlx::query(
                r#"
                UPDATE requests
                SET status = 'resolved',
                    resolved_at = ?,
                    updated_at = ?
                WHERE id = ? AND status = 'pending'
                "#,
            )
            .bind(&resolved_at_text)
            .bind(&resolved_at_text)
            .bind(request_id)
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query("UPDATE requests SET updated_at = ? WHERE id = ?")
                .bind(&resolved_at_text)
                .bind(request_id)
                .execute(&mut *transaction)
                .await?;
        }

        let resolved = get_request_on(&mut transaction, request_id).await?;
        transaction.commit().await?;
        return Ok(resolved);
    }

    let decision_json = serde_json::to_string(&decision)?;
    let resolved_at_text = resolved_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let updated = sqlx::query(
        r#"
        UPDATE requests
        SET status = 'resolved',
            decision_json = ?,
            resolved_at = ?,
            updated_at = ?
        WHERE id = ? AND status = 'pending'
        "#,
    )
    .bind(decision_json)
    .bind(&resolved_at_text)
    .bind(&resolved_at_text)
    .bind(request_id)
    .execute(&mut *transaction)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "request was already handled".to_string(),
        ));
    }

    let resolved = get_request_on(&mut transaction, request_id).await?;
    transaction.commit().await?;
    Ok(resolved)
}

async fn verified_decision_signature(
    connection: &mut SqliteConnection,
    request: &DecisionRequest,
    option: &RequestOption,
    actor_device: Option<&Device>,
    text: Option<&str>,
    provided: Option<&crate::models::SubmitDecisionSignature>,
) -> Result<Option<DecisionSignatureRecord>, ApiError> {
    let Some(actor_device) = actor_device else {
        if provided.is_some() {
            return Err(ApiError::BadRequest(
                "decision signatures require a device actor".to_string(),
            ));
        }
        return Ok(None);
    };

    let device_has_key = actor_device
        .signing_public_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    let Some(provided) = provided else {
        if device_has_key {
            return Err(ApiError::BadRequest(
                "signed decision payload is required for this device".to_string(),
            ));
        }
        return Ok(None);
    };

    let (request_digest, signing_payload) =
        signing::verify_decision_signature(request, option, actor_device, text, provided)?;
    // Record the nonce after verification so invalid attempts cannot burn a valid nonce.
    insert_decision_nonce(
        connection,
        &actor_device.id,
        &provided.key_id,
        &provided.nonce,
    )
    .await?;
    Ok(Some(DecisionSignatureRecord {
        key_id: provided.key_id.clone(),
        algorithm: provided.algorithm.clone(),
        nonce: provided.nonce.clone(),
        signed_at: provided.signed_at.clone(),
        request_digest,
        signing_payload,
        signature: provided.signature.clone(),
        verified: true,
        public_key: actor_device.signing_public_key.clone(),
    }))
}

async fn insert_decision_nonce(
    connection: &mut SqliteConnection,
    device_id: &str,
    key_id: &str,
    nonce: &str,
) -> Result<(), ApiError> {
    let inserted = sqlx::query(
        r#"
        INSERT OR IGNORE INTO decision_nonces (device_id, key_id, nonce, used_at)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(device_id)
    .bind(key_id)
    .bind(nonce)
    .bind(now_string())
    .execute(connection)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "decision signature nonce has already been used".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expired_submission_leaves_the_expiry_sweep_able_to_broadcast() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", directory.path().join("nod.sqlite").display());
        config.data_dir = directory.path().to_path_buf();
        let pool = crate::db::connect(&config).await.unwrap();
        let created = crate::db::create_request(
            &pool,
            serde_json::from_value(serde_json::json!({"title":"Expired"})).unwrap(),
            crate::db::CreateRequestMetadata::default(),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE requests SET expires_at='2000-01-01T00:00:00.000Z' WHERE id=?")
            .bind(&created.request_id)
            .execute(&pool)
            .await
            .unwrap();
        let result = record_decision(
            &pool,
            DecisionSubmission {
                request_id: &created.request_id,
                option_id: "dismiss",
                actor_device: None,
                actor_user_id: Some("owner"),
                decision: serde_json::from_value(serde_json::json!({})).unwrap(),
            },
        )
        .await;
        assert!(matches!(result, Err(ApiError::Conflict(_))));
        let expired = crate::db::expire_due_requests(&pool).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, created.request_id);
        assert!(crate::db::expire_due_requests(&pool)
            .await
            .unwrap()
            .is_empty());
    }
}
