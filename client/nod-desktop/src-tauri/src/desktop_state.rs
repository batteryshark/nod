use std::sync::Arc;

use crate::{notifier::DesktopNotifier, preferences::PreferenceStore};
use nod_client_core::NodClientRuntime;
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct DesktopState {
    pub(crate) runtime: Arc<Mutex<NodClientRuntime>>,
    pub(crate) preferences: Arc<Mutex<PreferenceStore>>,
    pub(crate) notifier: DesktopNotifier,
}

impl DesktopState {
    pub(crate) fn new(
        runtime: Arc<Mutex<NodClientRuntime>>,
        preferences: Arc<Mutex<PreferenceStore>>,
        notifier: DesktopNotifier,
    ) -> Self {
        Self {
            runtime,
            preferences,
            notifier,
        }
    }
}
