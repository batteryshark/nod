use std::{io::ErrorKind, time::Duration};

use anyhow::{anyhow, Error, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::time::Instant;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{error::ProtocolError, Error as WebSocketError, Message},
};

use crate::{
    api::ApiStatusError,
    models::{NotificationDeliveryMode, RequestStatus, SyncEnvelope, SyncPhase},
};

use super::{emit_to, snapshot::SnapshotContext, NodClientMessage, NodClientRuntime};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(50);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

impl NodClientRuntime {
    pub async fn connect_sync(&mut self) -> Result<()> {
        self.disconnect_sync().await;
        let context = self.snapshot_context().await?;
        self.sync_task = Some(tokio::spawn(run_sync_loop(context)));
        Ok(())
    }

    pub async fn disconnect_sync(&mut self) {
        if let Some(task) = self.sync_task.take() {
            task.abort();
            // Await cancellation before changing profiles so an old snapshot
            // can never write into the newly selected server's state.
            let _ = task.await;
        }
        self.reducer.lock().await.mark_sync_connected(false);
        self.emit_message(NodClientMessage::SyncStatus { connected: false })
            .await;
        self.emit_state().await;
    }
}

async fn run_sync_loop(context: SnapshotContext) {
    let mut has_connected = false;
    let mut failures = 0u32;
    loop {
        context.phase(SyncPhase::Connecting).await;
        let started = Instant::now();
        let result = run_connection(&context).await;
        if let Err(error) = &result {
            if is_unauthorized(error) {
                let (state, pending_ids) = {
                    let mut reducer = context.reducer.lock().await;
                    let pending_ids = reducer.pending_request_ids();
                    reducer.clear_loaded_data();
                    reducer.set_error(
                        "This device registration was revoked. Enroll again to reconnect.",
                    );
                    (reducer.state.clone(), pending_ids)
                };
                for request_id in pending_ids {
                    emit_to(
                        &context.tx,
                        NodClientMessage::NotificationRemoved {
                            server_id: context.server_id.clone(),
                            request_id,
                        },
                    )
                    .await;
                }
                emit_to(&context.tx, NodClientMessage::State(Box::new(state))).await;
                context.phase(SyncPhase::Revoked).await;
                emit_to(&context.tx, NodClientMessage::AuthRevoked {}).await;
                return;
            }
            if !is_expected_reconnect_error(error, has_connected) {
                emit_to(
                    &context.tx,
                    NodClientMessage::TransientError {
                        message: error.to_string(),
                    },
                )
                .await;
            }
        }
        has_connected |= context.reducer.lock().await.state.is_sync_connected;
        failures = if started.elapsed() >= HEARTBEAT_INTERVAL {
            0
        } else {
            failures.saturating_add(1)
        };
        context.phase(SyncPhase::Offline).await;
        tokio::time::sleep(reconnect_delay(failures)).await;
    }
}

fn reconnect_delay(failures: u32) -> Duration {
    let seconds = 1u64 << failures.min(5);
    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]) * 4;
    Duration::from_millis((seconds * 1000 + jitter).min(MAX_RECONNECT_DELAY.as_millis() as u64))
}

fn is_unauthorized(error: &Error) -> bool {
    if error
        .downcast_ref::<ApiStatusError>()
        .is_some_and(|error| error.status == reqwest::StatusCode::UNAUTHORIZED)
    {
        return true;
    }
    matches!(error.downcast_ref::<WebSocketError>(), Some(WebSocketError::Http(response)) if response.status().as_u16() == 401)
        || error.downcast_ref::<RegistrationRevoked>().is_some()
}

#[derive(Debug, thiserror::Error)]
#[error("This device registration was revoked.")]
struct RegistrationRevoked;

fn is_expected_reconnect_error(error: &Error, has_connected: bool) -> bool {
    let Some(websocket_error) = error.downcast_ref::<WebSocketError>() else {
        return false;
    };

    match websocket_error {
        WebSocketError::ConnectionClosed => true,
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        WebSocketError::Io(error) if has_connected => matches!(
            error.kind(),
            ErrorKind::ConnectionAborted
                | ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::NotConnected
                | ErrorKind::TimedOut
                | ErrorKind::UnexpectedEof
        ),
        _ => false,
    }
}

async fn run_connection(context: &SnapshotContext) -> Result<()> {
    let url = context.api.websocket_url()?;
    let (mut socket, _) =
        tokio::time::timeout(CONNECT_TIMEOUT, connect_async(url.as_str())).await??;
    // Subscribe before taking a snapshot. Events arriving during HTTP remain
    // buffered on the socket; the reducer ignores versions older than the snapshot.
    context.phase(SyncPhase::Reconciling).await;
    context.refresh().await?;
    context.phase(SyncPhase::Current).await;
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut received_at = Instant::now();
    loop {
        tokio::select! {
            incoming = socket.next() => {
                let Some(message) = incoming else { return Ok(()); };
                let message = message?;
                received_at = Instant::now();
                if matches!(message, Message::Close(_)) { return Ok(()); }
                if let Some(envelope) = envelope_from_message(message)? {
                    let should_resync = should_resync_after(&envelope);
                    apply_sync_envelope(context, envelope).await?;
                    if should_resync {
                        context.phase(SyncPhase::Reconciling).await;
                        context.refresh().await?;
                        context.phase(SyncPhase::Current).await;
                    }
                }
            }
            _ = heartbeat.tick() => {
                socket.send(Message::Ping(Vec::new())).await?;
            }
            _ = tokio::time::sleep_until(received_at + LIVENESS_TIMEOUT) => {
                return Err(anyhow!("No response from sync server; reconnecting."));
            }
        }
    }
}

fn envelope_from_message(message: Message) -> Result<Option<SyncEnvelope>> {
    if !message.is_text() && !message.is_binary() {
        return Ok(None);
    }
    let raw = message.into_data();
    if raw.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&raw).map(Some).map_err(|_| {
        anyhow!("Invalid sync message received; reconnecting to restore current state.")
    })
}

async fn apply_sync_envelope(context: &SnapshotContext, envelope: SyncEnvelope) -> Result<()> {
    let reducer = &context.reducer;
    let tx = &context.tx;
    let notification_removal = notification_removal_for(&envelope);
    let auth_revoked = is_current_device_revoked(context, &envelope).await;
    let delivery_mode = notification_delivery_mode_for(&envelope);

    let candidates = {
        let mut reducer = reducer.lock().await;
        if let Some(mode) = delivery_mode {
            reducer.set_notification_delivery_mode(mode);
        }
        reducer.apply_sync_envelope(envelope)
    };

    context.emit_candidates(candidates).await;
    if let Some(request_id) = notification_removal {
        emit_to(
            tx,
            NodClientMessage::NotificationRemoved {
                server_id: context.server_id.clone(),
                request_id,
            },
        )
        .await;
    }

    let state = reducer.lock().await.state.clone();
    emit_to(tx, NodClientMessage::State(Box::new(state))).await;

    if auth_revoked {
        return Err(RegistrationRevoked.into());
    }
    Ok(())
}

fn notification_removal_for(envelope: &SyncEnvelope) -> Option<String> {
    envelope
        .payload
        .request
        .as_ref()
        .filter(|request| request.status != RequestStatus::Pending)
        .map(|request| request.id.clone())
}

fn notification_delivery_mode_for(envelope: &SyncEnvelope) -> Option<NotificationDeliveryMode> {
    envelope
        .notification_delivery
        .as_ref()
        .or(envelope.payload.notification_delivery.as_ref())
        .map(|delivery| delivery.mode.clone())
}

async fn is_current_device_revoked(context: &SnapshotContext, envelope: &SyncEnvelope) -> bool {
    if envelope.kind != "device_revoked" {
        return false;
    }
    let Some(device_id) = envelope
        .payload
        .extra
        .get("device_id")
        .and_then(|value| value.as_str())
    else {
        return false;
    };

    context
        .reducer
        .lock()
        .await
        .selected_server()
        .and_then(|server| server.device_id.as_deref())
        == Some(device_id)
}

fn should_resync_after(envelope: &SyncEnvelope) -> bool {
    matches!(
        envelope.kind.as_str(),
        "cleared" | "subscription_updated" | "resync_required"
    ) || envelope.kind.starts_with("device_")
}

#[cfg(test)]
mod tests {
    use std::io;

    use anyhow::anyhow;
    use chrono::Utc;

    use super::*;
    use crate::models::{NotificationDelivery, NotificationDeliveryMode, SyncPayload};

    fn envelope(kind: &str) -> SyncEnvelope {
        SyncEnvelope {
            kind: kind.to_string(),
            at: Utc::now(),
            notification_delivery: None,
            payload: SyncPayload::default(),
        }
    }

    #[test]
    fn resyncs_after_server_requested_snapshot_changes() {
        assert!(should_resync_after(&envelope("cleared")));
        assert!(should_resync_after(&envelope("subscription_updated")));
        assert!(should_resync_after(&envelope("resync_required")));
    }

    #[test]
    fn resyncs_after_device_lifecycle_messages() {
        assert!(should_resync_after(&envelope("device_revoked")));
        assert!(should_resync_after(&envelope("device_renamed")));
    }

    #[test]
    fn does_not_resync_after_request_creation() {
        assert!(!should_resync_after(&envelope("created")));
    }

    #[test]
    fn reads_notification_delivery_from_hello_payload() {
        let mut envelope = envelope("hello");
        envelope.payload.notification_delivery = Some(NotificationDelivery {
            mode: NotificationDeliveryMode::Push,
        });

        assert_eq!(
            notification_delivery_mode_for(&envelope),
            Some(NotificationDeliveryMode::Push)
        );
    }

    #[test]
    fn treats_reset_without_close_handshake_as_reconnect_condition() {
        let error = Error::new(WebSocketError::Protocol(
            ProtocolError::ResetWithoutClosingHandshake,
        ));

        assert!(is_expected_reconnect_error(&error, false));
    }

    #[test]
    fn treats_refused_connection_after_success_as_reconnect_condition() {
        let error = Error::new(WebSocketError::Io(io::Error::from(
            ErrorKind::ConnectionRefused,
        )));

        assert!(is_expected_reconnect_error(&error, true));
    }

    #[test]
    fn surfaces_refused_connection_before_first_success() {
        let error = Error::new(WebSocketError::Io(io::Error::from(
            ErrorKind::ConnectionRefused,
        )));

        assert!(!is_expected_reconnect_error(&error, false));
    }

    #[test]
    fn surfaces_non_websocket_sync_errors() {
        let error = anyhow!("decode drift");

        assert!(!is_expected_reconnect_error(&error, true));
    }
}
