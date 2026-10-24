use crate::{
    auth, db,
    error::ApiError,
    models::{
        CreateDecisionRequest, CreatedDecisionRequest, DecisionRequest, Device, RequestStatus,
        SubmitDecisionRequest,
    },
    state::AppState,
    sync,
};
use std::time::Duration;
use tokio::time::timeout;

pub(crate) struct WaitForDecision {
    pub request: DecisionRequest,
    pub timed_out: bool,
}

pub(crate) async fn create(
    state: &AppState,
    request: CreateDecisionRequest,
    metadata: db::CreateRequestMetadata<'_>,
) -> Result<CreatedDecisionRequest, ApiError> {
    super::callback_delivery::validate_destination(
        state.config.callback_allowed_origins.as_deref(),
        request.callback_url.as_deref(),
    )?;
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
    // The canonical request drives audit and fanout: a shared resolution must
    // reach every recipient, so nothing here may see a per-user projection.
    // Callback obligations are frozen and queued in this DB transaction, not spawned here.
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
