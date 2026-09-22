use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{interval, Duration};

use crate::{auth, error::ApiError, state::AppState, sync};

#[derive(Debug, Deserialize)]
pub(super) struct SyncQuery {
    token: String,
}

pub(super) async fn sync_socket(
    State(state): State<AppState>,
    Query(query): Query<SyncQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let hash = auth::hash_secret(&query.token);
    let device = auth::find_device_by_hash(&state.pool, &hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(ws
        .on_upgrade(move |socket| handle_socket(socket, state, device.id, device.user_id))
        .into_response())
}

pub(super) async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    device_id: String,
    user_id: String,
) {
    let mut rx = state.sync.subscribe();
    let mut authorization_check = interval(Duration::from_secs(15));
    let (mut sender, mut receiver) = socket.split();
    let Ok(device) = crate::db::get_device(&state.pool, &device_id).await else {
        return;
    };
    let hello = sync_hello(&device_id, &state.delivery_for_device(&device));
    if let Ok(text) = serde_json::to_string(&hello) {
        if sender.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(envelope) => {
                        // The database is authoritative even if a revocation
                        // broadcast was missed or this client ignores it.
                        if !device_is_active(&state, &device_id).await {
                            let _ = sender.send(Message::Close(None)).await;
                            break;
                        }
                        if let Some(targets) = envelope.target_user_ids.as_ref() {
                            if !targets.iter().any(|target| target == &user_id) {
                                continue;
                            }
                        }
                        if let Some(request_id) = envelope.payload.get("request").and_then(|request| request.get("id")).and_then(Value::as_str) {
                            match crate::db::request_delivery_eligible(&state.pool, request_id, &user_id).await {
                                Ok(true) => {},
                                Ok(false) => continue,
                                Err(error) => { tracing::warn!(%error, "closing socket after delivery eligibility check failed"); break; }
                            }
                        }
                        let envelope = sync_envelope_for_user(envelope, &user_id);
                        match serde_json::to_string(&envelope) {
                            Ok(text) => {
                                if sender.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => tracing::error!(error = %err, "failed to serialize sync message"),
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        let envelope = sync::device_update("resync_required", json!({ "skipped": skipped }));
                        if let Ok(text) = serde_json::to_string(&envelope) {
                            if sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = authorization_check.tick() => {
                if !device_is_active(&state, &device_id).await {
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {},
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "websocket receive error");
                        break;
                    }
                }
            }
        }
    }
}

async fn device_is_active(state: &AppState, device_id: &str) -> bool {
    match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM devices WHERE id = ? AND revoked_at IS NULL)",
    )
    .bind(device_id)
    .fetch_one(&state.pool)
    .await
    {
        Ok(active) => active,
        Err(error) => {
            tracing::warn!(%error, device_id, "closing socket after authorization check failed");
            false
        }
    }
}

fn sync_hello(
    device_id: &str,
    notification_delivery: &crate::models::NotificationDelivery,
) -> crate::models::SyncEnvelope {
    sync::device_update(
        "hello",
        json!({ "device_id": device_id, "notification_delivery": notification_delivery }),
    )
}

fn sync_envelope_for_user(
    mut envelope: crate::models::SyncEnvelope,
    user_id: &str,
) -> crate::models::SyncEnvelope {
    let is_targeted = envelope.target_user_ids.is_some();
    if !is_targeted {
        return envelope;
    }
    envelope.target_user_ids = None;
    // Targeted fanout strips other recipients and per-user decisions before socket delivery.
    if let Some(request_value) = envelope.payload.get_mut("request") {
        filter_request_for_user(request_value, user_id);
    }
    envelope
}

fn filter_request_for_user(request: &mut Value, user_id: &str) {
    let Value::Object(object) = request else {
        return;
    };
    if object
        .get("recipients")
        .and_then(Value::as_array)
        .is_some_and(|recipients| recipients.len() > 1)
    {
        object.insert("request_digest".to_string(), Value::Null);
    }
    object.insert("recipients".to_string(), json!([user_id]));

    let decision_resolution = object
        .get("decision_resolution")
        .and_then(Value::as_str)
        .unwrap_or("shared");
    if decision_resolution != "per_user" {
        return;
    }

    let decisions = object
        .get("decisions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let user_decision = decisions
        .iter()
        .find(|decision| decision.get("user_id").and_then(Value::as_str) == Some(user_id))
        .cloned();
    object.insert(
        "decisions".to_string(),
        user_decision
            .as_ref()
            .map(|decision| json!([decision]))
            .unwrap_or_else(|| json!([])),
    );
    if let Some(decision) = user_decision.and_then(|value| value.get("decision").cloned()) {
        object.insert("status".to_string(), json!("resolved"));
        object.insert(
            "resolved_at".to_string(),
            decision.get("resolved_at").cloned().unwrap_or(Value::Null),
        );
        object.insert("decision".to_string(), decision);
    } else if object.get("status").and_then(Value::as_str) == Some("resolved") {
        object.insert("status".to_string(), json!("pending"));
        object.insert("resolved_at".to_string(), Value::Null);
        object.insert("decision".to_string(), Value::Null);
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{NotificationDelivery, NotificationDeliveryMode};

    use super::sync_hello;

    #[test]
    fn websocket_hello_includes_notification_delivery() {
        let hello = sync_hello(
            "device-1",
            &NotificationDelivery {
                mode: NotificationDeliveryMode::Websocket,
            },
        );

        assert_eq!(hello.kind, "hello");
        assert_eq!(hello.payload["device_id"], "device-1");
        assert_eq!(hello.payload["notification_delivery"]["mode"], "websocket");
    }
}
