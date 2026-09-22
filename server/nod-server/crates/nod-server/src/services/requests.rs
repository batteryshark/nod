use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use crate::{
    auth, db,
    error::ApiError,
    models::{
        CreateDecisionRequest, CreatedDecisionRequest, DecisionRequest, Device, RequestStatus,
        SubmitDecisionRequest,
    },
    state::AppState,
    sync,
    views::CallbackPayload,
};

pub(crate) struct WaitForDecision {
    pub request: DecisionRequest,
    pub timed_out: bool,
}

pub(crate) async fn create(
    state: &AppState,
    request: CreateDecisionRequest,
    metadata: db::CreateRequestMetadata<'_>,
) -> Result<CreatedDecisionRequest, ApiError> {
    validate_callback_destination(state, request.callback_url.as_deref())?;
    let response = db::create_request(&state.pool, request, metadata).await?;
    if response.deduped {
        state
            .audit
            .record("request.deduped", &response.request)
            .await;
    } else {
        state
            .audit
            .record("request.created", &response.request)
            .await;
        let _ = state.sync.send(sync::request("created", &response.request));
    }
    Ok(response)
}

pub(crate) async fn cancel(
    state: &AppState,
    request_id: &str,
) -> Result<DecisionRequest, ApiError> {
    let request = db::cancel_request(&state.pool, request_id).await?;
    state.audit.record("request.cancelled", &request).await;
    let _ = state.sync.send(sync::request("cancelled", &request));
    Ok(request)
}

pub(crate) async fn record_decision(
    state: &AppState,
    device: &Device,
    request_id: &str,
    option_id: &str,
    decision: SubmitDecisionRequest,
) -> Result<DecisionRequest, ApiError> {
    // The canonical request drives audit, fanout, and callbacks: a shared
    // resolution must reach every recipient (sync::request targets
    // request.recipients), so nothing here may see a per-user projection.
    let request = db::record_decision(
        &state.pool,
        db::DecisionSubmission {
            request_id,
            option_id,
            actor_device: Some(device),
            actor_user_id: Some(&device.user_id),
            decision,
        },
    )
    .await?;
    state.audit.record("decision.recorded", &request).await;
    let envelope = if request.decision_resolution == crate::models::DecisionResolution::PerUser {
        sync::request_for_users("resolved", &request, vec![device.user_id.clone()])
    } else {
        sync::request("resolved", &request)
    };
    let _ = state.sync.send(envelope);
    queue_callback(state, &request).await;
    // The actor's response is their own projection, same as any device read.
    db::request_for_user(&state.pool, request_id, &device.user_id).await
}

pub(crate) async fn request_for_principal(
    state: &AppState,
    principal: &auth::Principal,
    request_id: &str,
) -> Result<DecisionRequest, ApiError> {
    match principal {
        auth::Principal::Device(device) => {
            if !db::request_visible_to_user(&state.pool, request_id, &device.user_id).await? {
                return Err(ApiError::Forbidden);
            }
            db::request_for_user(&state.pool, request_id, &device.user_id).await
        }
        _ => {
            let request = db::get_request(&state.pool, request_id).await?;
            auth::require_request_read(principal, &request.channel_id)?;
            Ok(request)
        }
    }
}

pub(crate) async fn wait_for_decision(
    state: &AppState,
    request_id: &str,
    device_user_id: Option<&str>,
    wait_for: Duration,
) -> Result<WaitForDecision, ApiError> {
    let mut rx = state.sync.subscribe();
    let poll = async {
        loop {
            let request = if let Some(user_id) = device_user_id {
                db::request_for_user(&state.pool, request_id, user_id).await?
            } else {
                db::get_request(&state.pool, request_id).await?
            };
            if !matches!(request.status, RequestStatus::Pending) {
                return Ok::<_, ApiError>(request);
            }
            // Broadcasts are the fast path; polling keeps waits correct if a message is missed.
            tokio::select! {
                _ = rx.recv() => {},
                _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
    };

    match timeout(wait_for, poll).await {
        Ok(Ok(request)) => Ok(WaitForDecision {
            request,
            timed_out: false,
        }),
        Ok(Err(err)) => Err(err),
        Err(_) => {
            let request = if let Some(user_id) = device_user_id {
                db::request_for_user(&state.pool, request_id, user_id).await?
            } else {
                db::get_request(&state.pool, request_id).await?
            };
            Ok(WaitForDecision {
                request,
                timed_out: true,
            })
        }
    }
}

async fn dispatch_callback(state: &AppState, request: &DecisionRequest) {
    let Some(callback_url) = request.callback_url.as_deref() else {
        return;
    };
    if let Err(error) = validate_callback_destination(state, Some(callback_url)) {
        state
            .audit
            .record(
                "callback.blocked",
                &json!({"request_id":request.id,"error":error.to_string()}),
            )
            .await;
        return;
    }
    let payload = CallbackPayload::from_request(request);
    match state.http.post(callback_url).json(&payload).send().await {
        Ok(response) if response.status().is_success() => {
            state.audit.record("callback.delivered", &payload).await;
        }
        Ok(response) => {
            let status = response.status();
            let text = bounded_callback_response(response).await;
            tracing::warn!(%status, request_id = %request.id, "callback rejected");
            state
                .audit
                .record(
                    "callback.failed",
                    &json!({ "request_id": request.id, "status": status.as_u16(), "body": text }),
                )
                .await;
        }
        Err(err) => {
            let err = err.without_url();
            tracing::warn!(error = %err, request_id = %request.id, "callback failed");
            state
                .audit
                .record(
                    "callback.failed",
                    &json!({ "request_id": request.id, "error": err.to_string() }),
                )
                .await;
        }
    }
}

fn validate_callback_destination(state: &AppState, callback: Option<&str>) -> Result<(), ApiError> {
    let Some(callback) = callback else {
        return Ok(());
    };
    let url = url::Url::parse(callback)
        .map_err(|_| ApiError::BadRequest("invalid callback URL".to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::BadRequest(
            "callback URL must be HTTP(S) without userinfo".to_string(),
        ));
    }
    if let Some(allowed) = &state.config.callback_allowed_origins {
        let origin = url.origin();
        if !allowed
            .iter()
            .any(|value| url::Url::parse(value).is_ok_and(|allowed| allowed.origin() == origin))
        {
            return Err(ApiError::BadRequest(
                "callback origin is not allowed by this server".to_string(),
            ));
        }
    }
    Ok(())
}

async fn queue_callback(state: &AppState, request: &DecisionRequest) {
    if request.callback_url.is_none() {
        return;
    }
    let permit = match state.callback_slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            tracing::warn!(request_id = %request.id, "callback skipped: concurrency limit reached");
            state
                .audit
                .record(
                    "callback.failed",
                    &json!({"request_id":request.id,"error":"callback concurrency limit reached"}),
                )
                .await;
            return;
        }
    };
    let state = state.clone();
    let request = request.clone();
    tokio::spawn(async move {
        let _permit = permit;
        dispatch_callback(&state, &request).await;
    });
}

async fn bounded_callback_response(mut response: reqwest::Response) -> String {
    const MAX_RESPONSE_BYTES: usize = 4096;
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let remaining = MAX_RESPONSE_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() == MAX_RESPONSE_BYTES {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn callbacks_do_not_follow_redirects_and_diagnostics_are_bounded() {
        use axum::{response::Redirect, routing::post, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let target_calls = calls.clone();
        let app = Router::new()
            .route(
                "/redirect",
                post(|| async { Redirect::temporary("/target") }),
            )
            .route(
                "/target",
                post(move || {
                    let calls = target_calls.clone();
                    async move {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        "unexpected"
                    }
                }),
            )
            .route("/large", post(|| async { "x".repeat(100_000) }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", directory.path().join("nod.sqlite").display());
        config.data_dir = directory.path().to_path_buf();
        let state = AppState::new(config).await.unwrap();
        let redirect = state
            .http
            .post(format!("http://{address}/redirect"))
            .send()
            .await
            .unwrap();
        assert!(redirect.status().is_redirection());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let large = state
            .http
            .post(format!("http://{address}/large"))
            .send()
            .await
            .unwrap();
        assert_eq!(bounded_callback_response(large).await.len(), 4096);
        server.abort();
    }
}
