use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{State, WebSocketUpgrade},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::time::{interval, timeout, Duration};

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

use crate::{auth, error::ApiError, state::AppState, sync};

/// The device token travels only in the upgrade request's `Authorization:
/// Bearer` header, never in the URL: proxies and browsers log request lines,
/// and a device token is a long-lived credential. There is no query-string
/// fallback.
pub(super) async fn sync_socket(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let token = auth::bearer_token(&headers)?;
    let hash = auth::hash_secret(token);
    let device = auth::find_device_by_hash(&state.pool, &hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(ws
        .on_upgrade(move |socket| {
            handle_socket(
                socket,
                state,
                device.id,
                device.user_id,
                super::requests::value_capable(&headers),
            )
        })
        .into_response())
}

pub(super) async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    device_id: String,
    user_id: String,
    capable: bool,
) {
    let mut rx = state.sync.subscribe();
    let mut authorization_check = interval(Duration::from_secs(15));
    // Authorization is checked before hello below. Avoid an immediate extra
    // snapshot request: the first safety-net resync is due after 15 seconds.
    authorization_check.tick().await;
    let (mut sender, mut receiver) = socket.split();
    let Ok(device) = crate::db::get_device(&state.pool, &device_id).await else {
        return;
    };
    if !device_is_active(&state, &device_id).await {
        return;
    }
    let hello = sync_hello(&device_id, &state.delivery_for_device(&device));
    if !send_envelope(&mut sender, &hello).await {
        return;
    }

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(envelope) => {
                        // The database is authoritative even if a revocation
                        // broadcast was missed or this client ignores it.
                        if !device_is_active(&state, &device_id).await {
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
                        let mut envelope = sync_envelope_for_user(envelope, &user_id);
                        if !capable {
                            if let Some(request) = envelope.payload.get_mut("request") {
                                // Fail closed if an internal malformed request cannot be projected.
                                let Ok(wire) = serde_json::from_value::<nod_proto::Request>(request.clone()) else { break; };
                                *request = serde_json::to_value(wire.legacy_value_projection()).expect("wire serialization");
                            }
                        }
                        if !send_envelope(&mut sender, &envelope).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        if !device_is_active(&state, &device_id).await {
                            break;
                        }
                        let envelope = lagged_resync(skipped);
                        if !send_envelope(&mut sender, &envelope).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = authorization_check.tick() => {
                if !device_is_active(&state, &device_id).await {
                    break;
                }
                // A missing post-commit broadcast is not reported as Lagged.
                // Ask existing clients for an authoritative snapshot even on a quiet socket.
                let envelope = sync::device_update("resync_required", json!({}));
                if !send_envelope(&mut sender, &envelope).await {
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
    let _ = send_message(&mut sender, Message::Close(None)).await;
}

async fn send_message<S>(sender: &mut S, message: Message) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    matches!(
        timeout(WRITE_TIMEOUT, sender.send(message)).await,
        Ok(Ok(()))
    )
}

fn lagged_resync(skipped: u64) -> crate::models::SyncEnvelope {
    sync::device_update("resync_required", json!({ "skipped": skipped }))
}

async fn send_envelope<S, T>(sender: &mut S, envelope: &T) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
    T: Serialize,
{
    match serde_json::to_string(envelope) {
        Ok(text) => send_message(sender, Message::Text(text.into())).await,
        Err(error) => {
            tracing::error!(%error, "closing socket after sync serialization failed");
            false
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
    use axum::extract::ws::Message;
    use futures_util::sink;
    use serde_json::json;

    use super::sync_hello;

    #[tokio::test]
    async fn lagged_receiver_requests_recognized_content_free_recovery() {
        let tx = crate::sync::sender();
        let mut rx = tx.subscribe();
        for _ in 0..513 {
            tx.send(crate::sync::device_update(
                "private",
                json!({"secret": "other user"}),
            ))
            .unwrap();
        }
        let Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) = rx.recv().await else {
            panic!("expected lag past the 512-message buffer");
        };
        let recovery = super::lagged_resync(skipped);
        assert_eq!(recovery.kind, "resync_required");
        assert_eq!(recovery.payload, json!({"skipped": 1}));
        assert!(recovery.target_user_ids.is_none());
    }

    #[tokio::test]
    async fn stalled_writes_are_bounded_including_close() {
        // A deterministic non-draining sink avoids relying on OS TCP buffers.
        let mut stalled = Box::pin(sink::unfold((), |(), _: Message| {
            std::future::pending::<Result<(), std::io::Error>>()
        }));
        let start = tokio::time::Instant::now();
        assert!(!super::send_message(&mut stalled, Message::Close(None)).await);
        assert!(start.elapsed() < super::WRITE_TIMEOUT + std::time::Duration::from_secs(2));
    }

    #[tokio::test]
    async fn serialization_error_is_fatal() {
        struct Broken;
        impl serde::Serialize for Broken {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("injected"))
            }
        }
        let mut sender = sink::drain();
        assert!(!super::send_envelope(&mut sender, &Broken).await);
    }

    #[test]
    fn targeted_per_user_projection_does_not_leak_other_decisions() {
        let envelope = crate::sync::device_update(
            "request_updated",
            json!({"request": {
                "recipients": ["alice", "bob"], "request_digest": "private",
                "decision_resolution": "per_user", "status": "resolved",
                "decision": {"text": "bob secret"},
                "decisions": [{"user_id": "bob", "decision": {"text": "bob secret"}}]
            }}),
        );
        let mut envelope = envelope;
        envelope.target_user_ids = Some(vec!["alice".into(), "bob".into()]);
        let projected = super::sync_envelope_for_user(envelope, "alice");
        assert!(projected.target_user_ids.is_none());
        let request = &projected.payload["request"];
        assert_eq!(request["recipients"], json!(["alice"]));
        assert_eq!(request["decisions"], json!([]));
        assert_eq!(request["status"], "pending");
        assert!(request["decision"].is_null());
        assert!(request["request_digest"].is_null());
        assert!(!serde_json::to_string(&projected).unwrap().contains("bob"));
    }

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
