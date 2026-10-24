use crate::{auth, db::lifecycle, error::ApiError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// Explicit legacy-ledger migration, never inferred from card content. It does
/// not rewrite historical event detail or fabricate a domain mutation.
pub(super) async fn bind_origin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(origin): Json<lifecycle::Origin>,
) -> Result<Json<Value>, ApiError> {
    origin.validate()?;
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    if matches!(principal, auth::Principal::Device(_)) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT channel_id,status,created_by_issuer_token_id FROM requests WHERE id=?",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (channel, status, creator) = row.ok_or(ApiError::NotFound)?;
    auth::require_request_write(&principal, &channel)?;
    if let auth::Principal::Issuer(token) = &principal {
        if creator.as_deref() != Some(token.id.as_str()) {
            return Err(ApiError::Forbidden);
        }
    }
    if status != "pending" {
        return Err(ApiError::Conflict(
            "origin migration requires pending request".into(),
        ));
    }
    if let Some(existing) = lifecycle::origin_on(&mut tx, &id).await? {
        if existing != origin {
            return Err(ApiError::Conflict("origin is immutable".into()));
        }
    } else {
        sqlx::query(
            "INSERT INTO request_origins(request_id,session,directory,reference) VALUES(?,?,?,?)",
        )
        .bind(&id)
        .bind(&origin.session)
        .bind(&origin.directory)
        .bind(&origin.reference)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"request_id":id,"origin":origin})))
}

#[derive(Deserialize)]
pub(super) struct ReplayQuery {
    after: Option<i64>,
    limit: Option<i64>,
    through: Option<i64>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReplayQuery>,
) -> Result<Json<Value>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    if matches!(principal, auth::Principal::Device(_)) {
        return Err(ApiError::Forbidden);
    }
    let after = query.after.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let high = lifecycle::high_water(&state.pool).await?;
    let through = query.through.unwrap_or(high);
    if after < 0 || !(1..=500).contains(&limit) || through < after || through > high {
        return Err(ApiError::BadRequest(
            "invalid replay cursor or limit".into(),
        ));
    }
    // Enforce read capability even for an empty journal (channel scoping is checked per row).
    if let auth::Principal::Issuer(token) = &principal {
        if !token.scopes.iter().any(|scope| {
            scope == "*"
                || scope == "requests:*"
                || scope == "requests:read"
                || scope == "requests:write"
                || scope.starts_with("requests:read:")
                || scope.starts_with("requests:write:")
                || scope.starts_with("requests:*:")
        }) {
            return Err(ApiError::Forbidden);
        }
    }
    let rows = lifecycle::scan(&state.pool, after, through, limit).await?;
    let cursor = rows.last().map(|(e, _)| e.seq).unwrap_or(through);
    let events: Vec<_> = rows
        .into_iter()
        .filter_map(|(event, channel)| {
            auth::require_request_read(&principal, &channel)
                .ok()
                .map(|_| event)
        })
        .collect();
    Ok(Json(
        json!({"events":events,"next_cursor":cursor,"high_water":through}),
    ))
}

pub(super) async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let principal = auth::authenticate(&headers, &state.pool, state.config.admin_token()).await?;
    if matches!(principal, auth::Principal::Device(_)) {
        return Err(ApiError::Forbidden);
    }
    let (channel, detail) = lifecycle::detail(&state.pool, &id).await?;
    auth::require_request_read(&principal, &channel)?;
    Ok(Json(detail))
}
