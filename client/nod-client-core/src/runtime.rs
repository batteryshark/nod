mod rpc;
mod session;
mod snapshot;
mod sync;
#[cfg(test)]
mod tests;
mod workflows;

use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

use crate::{
    models::{ClientState, Request},
    signing::ForeignSigner,
    state::StateReducer,
    store::{PersistedConfig, Store},
};

pub use rpc::{
    ChannelParams, DeviceNotificationPreferenceParams, EnrollParams, NotificationPreferenceParams,
    OpenRequestParams, QueryHistoryParams, RegisterPushTokenParams, RenameDeviceParams,
    RevokeDeviceParams, RpcRequest, RpcResponse, SelectRequestParams, SelectServerParams,
    SetSubscriptionParams, SubmitOptionParams, SubmitRequestOptionParams,
};

const DEFAULT_NOTIFICATION_SOUND: &str = "default";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum NodClientMessage {
    Ready {
        state_path: String,
    },
    State(Box<ClientState>),
    NotificationCandidate {
        server_id: String,
        request: Box<Request>,
    },
    NotificationRemoved {
        server_id: String,
        request_id: String,
    },
    SyncStatus {
        connected: bool,
    },
    AuthRevoked {},
    ResyncRequired {},
    TransientError {
        message: String,
    },
}

type Outbox = mpsc::Sender<NodClientMessage>;

/// Where the runtime gets device signing keys. The TUI + desktop use software
/// keys persisted in the `Store`; Apple injects a `Foreign` backend so signing
/// happens in the Secure Enclave and no private key is ever persisted by Rust.
pub enum SignerBackend {
    /// Software P-256 keys generated and stored by nod-client-core.
    Software,
    /// Host-owned hardware keys (Apple Secure Enclave) reached via a callback.
    Foreign(Arc<dyn ForeignSigner>),
}

pub struct NodClientRuntime {
    store: Store,
    persisted: Arc<Mutex<PersistedConfig>>,
    reducer: Arc<Mutex<StateReducer>>,
    tx: Outbox,
    sync_task: Option<JoinHandle<()>>,
    signer_backend: SignerBackend,
    http_client: reqwest::Client,
    snapshot_lock: Arc<Mutex<()>>,
}

impl NodClientRuntime {
    /// Construct with the default software signing backend (TUI + desktop).
    pub async fn new(tx: Outbox) -> Result<Self> {
        Self::with_signer_backend(tx, SignerBackend::Software).await
    }

    /// Construct with an explicit signing backend. Apple passes
    /// `SignerBackend::Foreign(..)` so decisions are signed in the Secure
    /// Enclave instead of by a software key.
    pub async fn with_signer_backend(tx: Outbox, signer_backend: SignerBackend) -> Result<Self> {
        Self::with_store(tx, signer_backend, Store::new()?).await
    }

    async fn with_store(tx: Outbox, signer_backend: SignerBackend, store: Store) -> Result<Self> {
        let mut persisted = store.load().await?;
        normalize_notification_sound(&mut persisted);
        if migrate_profile_ids(&mut persisted) {
            store.save(persisted.clone()).await?;
        }

        let selected_server_id = selected_server_id_for(&persisted);
        persisted.selected_server_id = selected_server_id.clone();
        let mut reducer = StateReducer::new(
            persisted.servers.clone(),
            selected_server_id,
            persisted.notification_sound.clone(),
        );

        if let Some(warning) = store.recovery_warning() {
            reducer.set_error(warning);
        }

        Ok(Self {
            store,
            persisted: Arc::new(Mutex::new(persisted)),
            reducer: Arc::new(Mutex::new(reducer)),
            tx,
            sync_task: None,
            signer_backend,
            http_client: crate::api::NodApi::http_client()?,
            snapshot_lock: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn signer_backend(&self) -> &SignerBackend {
        &self.signer_backend
    }

    pub async fn emit_ready(&self) {
        self.emit_message(NodClientMessage::Ready {
            state_path: self.store.path().display().to_string(),
        })
        .await;
    }

    pub async fn state(&self) -> ClientState {
        self.reducer.lock().await.state.clone()
    }

    pub async fn emit_state(&self) {
        self.emit_message(NodClientMessage::State(Box::new(self.state().await)))
            .await;
    }

    async fn emit_message(&self, message: NodClientMessage) {
        emit_to(&self.tx, message).await;
    }
}

fn migrate_profile_ids(config: &mut PersistedConfig) -> bool {
    let mut changed = false;
    for server in &mut config.servers {
        let id = crate::api::profile_id_for(&server.base_url_string);
        if server.id == id {
            continue;
        }
        if config.selected_server_id.as_deref() == Some(&server.id) {
            config.selected_server_id = Some(id.clone());
        }
        server
            .credential_id
            .get_or_insert_with(|| server.id.clone());
        server.id = id;
        changed = true;
    }
    changed
}

async fn emit_to(tx: &Outbox, message: NodClientMessage) {
    let _ = tx.send(message).await;
}

fn normalize_notification_sound(config: &mut PersistedConfig) {
    if config.notification_sound.trim().is_empty() {
        config.notification_sound = DEFAULT_NOTIFICATION_SOUND.to_string();
    }
}

fn selected_server_id_for(config: &PersistedConfig) -> Option<String> {
    config
        .selected_server_id
        .clone()
        .filter(|id| config.servers.iter().any(|server| server.id == *id))
        .or_else(|| config.servers.first().map(|server| server.id.clone()))
}
