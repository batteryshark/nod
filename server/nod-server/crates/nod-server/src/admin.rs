use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{auth, error::ApiError, state::AppState};

/// Baked into the binary so a downloaded release runs with no asset files on disk.
const ADMIN_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/admin.html"
));

#[derive(Debug, Deserialize)]
pub(crate) struct AdminSessionRequest {
    token: String,
}

#[derive(Debug, Serialize)]
struct AdminSessionResponse {
    ok: bool,
}

impl AdminSessionResponse {
    fn ok() -> Self {
        Self { ok: true }
    }
}

pub(crate) async fn admin_page() -> Result<Response, ApiError> {
    // NOD_ADMIN_HTML_PATH is read per request so admin-panel edits show on
    // refresh during development without rebuilding the embedded copy. A set
    // path that fails to read is a loud 500, not a silent fall back to the
    // embedded copy — serving stale content would defeat the live-edit knob.
    if let Some(path) = std::env::var("NOD_ADMIN_HTML_PATH")
        .ok()
        .filter(|path| !path.is_empty())
    {
        let html = tokio::fs::read_to_string(&path).await.map_err(|err| {
            tracing::error!(
                path = %path,
                error = %err,
                "failed to read NOD_ADMIN_HTML_PATH override"
            );
            ApiError::Internal("admin HTML override unavailable".to_string())
        })?;
        return Ok(admin_html_response(html));
    }

    Ok(admin_html_response(ADMIN_HTML.to_string()))
}

// Hash the trusted template's inline script so release and live-development
// assets share the same policy without permitting injected inline scripts.
fn admin_html_response(html: String) -> Response {
    let hashes = html
        .split("<script>")
        .skip(1)
        .filter_map(|part| part.split_once("</script>"))
        .map(|(script, _)| {
            format!(
                "'sha256-{}'",
                STANDARD.encode(Sha256::digest(script.as_bytes()))
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let policy = format!("default-src 'none'; script-src {hashes}; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'");
    (
        [
            (header::CONTENT_SECURITY_POLICY, policy),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::REFERRER_POLICY, "no-referrer".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Html(html),
    )
        .into_response()
}

pub(crate) async fn create_admin_session(
    State(state): State<AppState>,
    Json(req): Json<AdminSessionRequest>,
) -> Result<Response, ApiError> {
    if !auth::admin_token_matches(req.token.trim(), state.config.admin_token()) {
        return Err(ApiError::Forbidden);
    }

    let cookie = auth::create_admin_session_cookie(state.config.admin_token());
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(AdminSessionResponse::ok()),
    )
        .into_response())
}

pub(crate) async fn delete_admin_session() -> Response {
    (
        [(header::SET_COOKIE, auth::expired_admin_session_cookie())],
        Json(AdminSessionResponse::ok()),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_policy_only_authorizes_embedded_script_and_same_origin_connections() {
        let response = admin_html_response(ADMIN_HTML.to_string());
        let policy = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        let script_policy = policy
            .split(';')
            .find(|part| part.trim_start().starts_with("script-src"))
            .unwrap();
        assert!(script_policy.contains("'sha256-"));
        assert!(!script_policy.contains("unsafe-inline"));
        assert!(policy.contains("connect-src 'self'"));
        assert!(!ADMIN_HTML.contains("<script src="));
        assert!(!ADMIN_HTML.contains("cdn.jsdelivr.net"));
    }
}
