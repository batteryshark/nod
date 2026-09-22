use std::collections::HashMap;

use sqlx::{sqlite::SqliteRow, Row, SqliteConnection};

use super::super::validation::{parse_optional_time, parse_time};
use crate::{
    error::ApiError,
    models::{
        CardField, CardLink, Decision, DecisionRequest, DecisionResolution, RequestNotification,
        RequestOption, RequestStatus, UserDecision,
    },
};

#[derive(Default)]
struct RelatedRows {
    recipients: Vec<String>,
    decisions: Vec<UserDecision>,
    options: Vec<RequestOption>,
}

pub(super) async fn row_to_request(
    connection: &mut SqliteConnection,
    row: SqliteRow,
) -> Result<DecisionRequest, ApiError> {
    rows_to_requests(connection, vec![row])
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::NotFound)
}

pub(super) async fn rows_to_requests(
    connection: &mut SqliteConnection,
    rows: Vec<SqliteRow>,
) -> Result<Vec<DecisionRequest>, ApiError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows.iter().map(|row| row.get("id")).collect();
    let ids_json = serde_json::to_string(&ids)?;
    let mut related = HashMap::<String, RelatedRows>::new();
    // One bound JSON array avoids both N+1 queries and SQLite's bind-variable
    // limit when a device has a large pending inbox.
    for row in sqlx::query("SELECT request_id,user_id FROM request_recipients WHERE request_id IN (SELECT value FROM json_each(?)) ORDER BY rowid")
        .bind(&ids_json).fetch_all(&mut *connection).await? {
        related.entry(row.get("request_id")).or_default().recipients.push(row.get("user_id"));
    }
    for row in sqlx::query("SELECT request_id,user_id,decision_json FROM request_user_decisions WHERE request_id IN (SELECT value FROM json_each(?)) ORDER BY resolved_at,user_id")
        .bind(&ids_json).fetch_all(&mut *connection).await? {
        let decision_json: String = row.get("decision_json");
        related.entry(row.get("request_id")).or_default().decisions.push(UserDecision {
            user_id: row.get("user_id"), decision: serde_json::from_str::<Decision>(&decision_json)?,
        });
    }
    for row in sqlx::query("SELECT * FROM request_options WHERE request_id IN (SELECT value FROM json_each(?)) ORDER BY rowid")
        .bind(&ids_json).fetch_all(&mut *connection).await? {
        related.entry(row.get("request_id")).or_default().options.push(RequestOption {
            id: row.get("option_id"), label: row.get("label"),
            kind: crate::models::OptionKind::from(row.get::<String, _>("kind").as_str()),
            style: row.get("style"), requires_text: row.get::<i64, _>("requires_text") == 1,
            text_placeholder: row.get("text_placeholder"), destructive: row.get::<i64, _>("destructive") == 1,
            foreground: row.get::<i64, _>("foreground") == 1,
        });
    }
    rows.into_iter()
        .map(|row| {
            let id: String = row.get("id");
            request_from_rows(row, related.remove(&id).unwrap_or_default())
        })
        .collect()
}

fn request_from_rows(row: SqliteRow, related: RelatedRows) -> Result<DecisionRequest, ApiError> {
    let fields_json: String = row.get("fields_json");
    let links_json: String = row.get("links_json");
    let notification_json: String = row.get("notification_json");
    let decision_json: Option<String> = row.get("decision_json");

    let salt: String = row.get("recipient_salt");
    let commitment = nod_proto::recipients_commitment(&related.recipients, &salt)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut request = DecisionRequest {
        id: row.get("id"),
        channel_id: row.get("channel_id"),
        recipients: related.recipients,
        decision_resolution: DecisionResolution::from(
            row.get::<String, _>("decision_resolution").as_str(),
        ),
        title: row.get("title"),
        summary: row.get("summary"),
        body_markdown: row.get("body_markdown"),
        fields: serde_json::from_str::<Vec<CardField>>(&fields_json)?,
        links: serde_json::from_str::<Vec<CardLink>>(&links_json)?,
        image_url: row.get("image_url"),
        notification: serde_json::from_str::<RequestNotification>(&notification_json)
            .unwrap_or_default(),
        dedupe_key: row.get("dedupe_key"),
        expires_at: parse_optional_time(row.get("expires_at"))?,
        status: RequestStatus::from(row.get::<String, _>("status").as_str()),
        created_at: parse_time(row.get("created_at"))?,
        updated_at: parse_time(row.get("updated_at"))?,
        resolved_at: parse_optional_time(row.get("resolved_at"))?,
        decision: decision_json
            .map(|value| serde_json::from_str::<Decision>(&value))
            .transpose()?,
        user_decisions: related.decisions,
        callback_url: row.get("callback_url"),
        options: related.options,
        private_recipients: false,
        signing: None,
    };
    let digest = nod_proto::request_digest_v2(&request.to_wire(), &commitment)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    request.signing = Some(nod_proto::RequestSigningContext {
        version: nod_proto::PRIVATE_REQUEST_DIGEST_VERSION.to_string(),
        recipients_commitment: commitment,
        request_digest: digest,
    });
    Ok(request)
}
