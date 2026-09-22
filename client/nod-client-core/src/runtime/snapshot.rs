use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use super::{emit_to, NodClientMessage, Outbox};
use crate::{
    api::NodApi,
    models::{ClientState, Request, SyncPhase},
    state::StateReducer,
};

const REFRESH_HISTORY_LIMIT: usize = 500;

#[derive(Clone)]
pub(super) struct SnapshotContext {
    pub api: NodApi,
    pub server_id: String,
    pub reducer: Arc<Mutex<StateReducer>>,
    pub tx: Outbox,
    pub lock: Arc<Mutex<()>>,
}

impl SnapshotContext {
    pub async fn refresh(&self) -> Result<ClientState> {
        let _snapshot = self.lock.lock().await;
        let revision = self.reducer.lock().await.revision();
        let (current_user, mut devices, channels, requests) = tokio::try_join!(
            self.api.current_user(),
            self.api.devices(),
            self.api.channels(),
            self.api.requests(None, Some(REFRESH_HISTORY_LIMIT)),
        )?;
        devices.retain(|device| device.id != current_user.current_device.id);
        devices.insert(0, current_user.current_device.clone());
        let (candidates, removed, state) = {
            let mut reducer = self.reducer.lock().await;
            let old_pending = reducer.pending_request_ids();
            reducer.set_notification_delivery_mode(current_user.notification_delivery.mode);
            reducer.state.notification_sound = current_user.current_device.notification_sound;
            let candidates = reducer.apply_snapshot(
                Some(current_user.user),
                devices,
                channels,
                requests,
                revision,
            );
            let pending = reducer.pending_request_ids();
            let removed = old_pending
                .difference(&pending)
                .cloned()
                .collect::<Vec<_>>();
            (candidates, removed, reducer.state.clone())
        };
        self.emit_candidates(candidates).await;
        for request_id in removed {
            emit_to(
                &self.tx,
                NodClientMessage::NotificationRemoved {
                    server_id: self.server_id.clone(),
                    request_id,
                },
            )
            .await;
        }
        emit_to(&self.tx, NodClientMessage::State(Box::new(state.clone()))).await;
        Ok(state)
    }

    pub async fn emit_candidates(&self, candidates: Vec<Request>) {
        let preferences = {
            let reducer = self.reducer.lock().await;
            let device_id = reducer
                .selected_server()
                .and_then(|server| server.device_id.as_deref());
            reducer
                .state
                .devices
                .iter()
                .find(|device| Some(device.id.as_str()) == device_id)
                .map(|device| device.notification_preferences.clone())
                .unwrap_or_default()
        };
        for mut request in candidates {
            if !preferences.allows_alert(&request.channel_id, chrono::Utc::now()) {
                continue;
            }
            if preferences.hide_content {
                request.notification.redact = true;
                request.notification.title = None;
                request.notification.body = None;
            }
            emit_to(
                &self.tx,
                NodClientMessage::NotificationCandidate {
                    server_id: self.server_id.clone(),
                    request: Box::new(request),
                },
            )
            .await;
        }
    }

    pub async fn phase(&self, phase: SyncPhase) {
        let state = {
            let mut reducer = self.reducer.lock().await;
            reducer.set_sync_phase(phase);
            reducer.state.clone()
        };
        emit_to(
            &self.tx,
            NodClientMessage::SyncStatus {
                connected: state.is_sync_connected,
            },
        )
        .await;
        emit_to(&self.tx, NodClientMessage::State(Box::new(state))).await;
    }
}
