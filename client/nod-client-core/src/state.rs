use std::collections::{BTreeMap, BTreeSet};

use crate::models::{
    Channel, ClientState, NotificationDeliveryMode, Request, RequestStatus, ServerProfile,
    SyncEnvelope, SyncPhase, User, UserDevice,
};

const HANDLED_REQUEST_DISPLAY_LIMIT: usize = 500;
const SYNC_KIND_CREATED: &str = "created";

#[derive(Debug, Clone)]
pub struct StateReducer {
    pub state: ClientState,
    known_pending_request_channels: BTreeMap<String, String>,
    has_loaded_pending_snapshot: bool,
    requests_by_id: BTreeMap<String, Request>,
    revision: u64,
    request_revisions: BTreeMap<String, u64>,
}

impl StateReducer {
    pub fn new(
        servers: Vec<ServerProfile>,
        selected_server_id: Option<String>,
        notification_sound: String,
    ) -> Self {
        let is_registered = !servers.is_empty();
        Self {
            state: ClientState {
                servers,
                selected_server_id,
                current_user: None,
                devices: Vec::new(),
                channels: Vec::new(),
                pending_counts_by_channel: BTreeMap::new(),
                requests: Vec::new(),
                selected_channel_id: None,
                selected_request_id: None,
                notification_sound,
                notification_delivery_mode: NotificationDeliveryMode::Websocket,
                is_registered,
                is_sync_connected: false,
                sync_phase: SyncPhase::Offline,
                last_synced_at: None,
                last_error: None,
            },
            known_pending_request_channels: BTreeMap::new(),
            has_loaded_pending_snapshot: false,
            requests_by_id: BTreeMap::new(),
            revision: 0,
            request_revisions: BTreeMap::new(),
        }
    }

    pub fn selected_server(&self) -> Option<&ServerProfile> {
        self.state
            .selected_server_id
            .as_deref()
            .and_then(|id| self.state.servers.iter().find(|server| server.id == id))
            .or_else(|| self.state.servers.first())
    }

    pub fn upsert_server(&mut self, server: ServerProfile) {
        if let Some(existing) = self
            .state
            .servers
            .iter_mut()
            .find(|existing| existing.id == server.id)
        {
            *existing = server;
        } else {
            self.state.servers.push(server);
        }
        self.state.is_registered = !self.state.servers.is_empty();
    }

    pub fn remove_server(&mut self, server_id: &str) {
        self.state.servers.retain(|server| server.id != server_id);
        if self.state.selected_server_id.as_deref() == Some(server_id) {
            self.state.selected_server_id =
                self.state.servers.first().map(|server| server.id.clone());
            self.clear_loaded_data();
        }
        self.state.is_registered = !self.state.servers.is_empty();
    }

    pub fn set_selected_server(&mut self, server_id: String) {
        self.state.selected_server_id = Some(server_id);
        self.clear_loaded_data();
    }

    #[cfg(test)]
    pub fn apply_refresh(
        &mut self,
        current_user: Option<User>,
        devices: Vec<UserDevice>,
        channels: Vec<Channel>,
        requests: Vec<Request>,
    ) -> Vec<Request> {
        self.apply_snapshot(current_user, devices, channels, requests, self.revision)
    }

    pub fn pending_request_ids(&self) -> BTreeSet<String> {
        self.known_pending_request_channels
            .keys()
            .cloned()
            .collect()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn apply_snapshot(
        &mut self,
        current_user: Option<User>,
        devices: Vec<UserDevice>,
        channels: Vec<Channel>,
        requests: Vec<Request>,
        started_at_revision: u64,
    ) -> Vec<Request> {
        self.state.current_user = current_user;
        self.state.devices = devices;
        self.state.channels = channels;
        self.ensure_selected_channel();

        let mut snapshot: BTreeMap<_, _> = requests
            .into_iter()
            .map(|request| (request.id.clone(), request))
            .collect();
        // A submission or live event may finish while HTTP is in flight. Keep
        // those later mutations instead of letting an older snapshot undo them.
        for (id, request) in &self.requests_by_id {
            let changed_during_fetch =
                self.request_revisions.get(id).copied().unwrap_or(0) > started_at_revision;
            let snapshot_is_newer = snapshot
                .get(id)
                .is_some_and(|current| request_is_stale(request, current));
            if changed_during_fetch && !snapshot_is_newer {
                snapshot.insert(id.clone(), request.clone());
            }
        }
        self.requests_by_id = snapshot;
        self.request_revisions
            .retain(|id, _| self.requests_by_id.contains_key(id));
        let pending_requests =
            pending_requests(&self.requests_by_id.values().cloned().collect::<Vec<_>>());
        self.state.pending_counts_by_channel = count_pending_by_channel(&pending_requests);
        let notification_candidates = self.notification_candidates_after_refresh(&pending_requests);
        self.remember_pending_requests(&pending_requests);

        self.update_visible_requests();
        self.ensure_selected_request();
        self.state.last_error = None;
        self.state.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
        notification_candidates
    }

    pub fn apply_sync_envelope(&mut self, envelope: SyncEnvelope) -> Vec<Request> {
        self.state.is_sync_connected = true;
        let mut notification_candidates = Vec::new();
        if let Some(channel) = envelope.payload.channel {
            self.upsert_channel(channel);
        }
        if let Some(request) = envelope.payload.request {
            let is_new_pending = self.apply_request_update(request.clone());
            let should_notify = envelope.kind == SYNC_KIND_CREATED && is_new_pending;
            if should_notify {
                notification_candidates.push(request);
            }
        }
        notification_candidates
    }

    pub fn apply_request_update(&mut self, request: Request) -> bool {
        if self
            .requests_by_id
            .get(&request.id)
            .is_some_and(|existing| request_is_stale(&request, existing))
        {
            return false;
        }
        let is_new_pending = self.update_pending_tracking(&request);
        self.revision += 1;
        self.request_revisions
            .insert(request.id.clone(), self.revision);
        self.requests_by_id.insert(request.id.clone(), request);
        self.update_visible_requests();
        is_new_pending
    }

    fn update_visible_requests(&mut self) {
        self.state.requests = self
            .visible_requests_for_selected_channel(self.requests_by_id.values().cloned().collect());
        self.ensure_selected_request();
    }

    pub fn select_channel(&mut self, channel_id: Option<String>) {
        self.state.selected_channel_id = channel_id;
        self.update_visible_requests();
    }

    fn upsert_channel(&mut self, channel: Channel) {
        if let Some(existing) = self
            .state
            .channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            *existing = channel;
        } else {
            self.state.channels.push(channel);
        }
    }

    fn update_pending_tracking(&mut self, request: &Request) -> bool {
        let previous_channel_id = self.previous_pending_channel(request).map(str::to_string);
        let is_pending = request.status == RequestStatus::Pending;

        match (previous_channel_id, is_pending) {
            (None, true) => {
                self.mark_pending(request);
                true
            }
            (Some(previous_channel_id), false) => {
                self.clear_pending(&request.id, &previous_channel_id);
                false
            }
            (Some(previous_channel_id), true) => {
                self.update_pending_channel(&previous_channel_id, request);
                false
            }
            (None, false) => false,
        }
    }

    pub fn mark_sync_connected(&mut self, connected: bool) {
        self.state.is_sync_connected = connected;
        if !connected && self.state.sync_phase != SyncPhase::Revoked {
            self.state.sync_phase = SyncPhase::Offline;
        }
    }

    pub fn set_sync_phase(&mut self, phase: SyncPhase) {
        self.state.is_sync_connected = phase == SyncPhase::Current;
        self.state.sync_phase = phase;
    }

    pub fn set_notification_delivery_mode(&mut self, mode: NotificationDeliveryMode) {
        self.state.notification_delivery_mode = mode;
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.state.last_error = Some(error.into());
    }

    pub fn clear_loaded_data(&mut self) {
        self.state.current_user = None;
        self.state.devices.clear();
        self.state.channels.clear();
        self.state.pending_counts_by_channel.clear();
        self.state.requests.clear();
        self.state.selected_channel_id = None;
        self.state.selected_request_id = None;
        self.known_pending_request_channels.clear();
        self.has_loaded_pending_snapshot = false;
        self.requests_by_id.clear();
        self.request_revisions.clear();
        self.state.last_synced_at = None;
    }

    fn ensure_selected_channel(&mut self) {
        if self.state.selected_channel_id.is_none() {
            return;
        }
        let visible_channel_ids: BTreeSet<_> = self
            .state
            .channels
            .iter()
            .filter(|channel| channel.subscribed)
            .map(|channel| channel.id.as_str())
            .collect();
        let selection_is_visible = self
            .state
            .selected_channel_id
            .as_deref()
            .map(|id| visible_channel_ids.contains(id))
            .unwrap_or(false);

        if !selection_is_visible {
            self.state.selected_channel_id = None;
        }
    }

    fn notification_candidates_after_refresh(&self, pending_requests: &[Request]) -> Vec<Request> {
        // The first refresh is a baseline snapshot. Only later refreshes should
        // generate local notifications for newly observed pending requests.
        if !self.has_loaded_pending_snapshot {
            return Vec::new();
        }

        pending_requests
            .iter()
            .filter(|request| {
                !self
                    .known_pending_request_channels
                    .contains_key(&request.id)
            })
            .cloned()
            .collect()
    }

    fn remember_pending_requests(&mut self, pending_requests: &[Request]) {
        self.known_pending_request_channels = pending_request_channels(pending_requests);
        self.has_loaded_pending_snapshot = true;
    }

    fn previous_pending_channel(&self, request: &Request) -> Option<&str> {
        self.known_pending_request_channels
            .get(&request.id)
            .map(String::as_str)
            .or_else(|| {
                self.state
                    .requests
                    .iter()
                    .find(|existing| {
                        existing.id == request.id && existing.status == RequestStatus::Pending
                    })
                    .map(|existing| existing.channel_id.as_str())
            })
    }

    fn mark_pending(&mut self, request: &Request) {
        increment_pending_count(
            &mut self.state.pending_counts_by_channel,
            &request.channel_id,
        );
        self.known_pending_request_channels
            .insert(request.id.clone(), request.channel_id.clone());
    }

    fn clear_pending(&mut self, request_id: &str, channel_id: &str) {
        decrement_pending_count(&mut self.state.pending_counts_by_channel, channel_id);
        self.known_pending_request_channels.remove(request_id);
    }

    fn update_pending_channel(&mut self, previous_channel_id: &str, request: &Request) {
        if previous_channel_id != request.channel_id {
            decrement_pending_count(
                &mut self.state.pending_counts_by_channel,
                previous_channel_id,
            );
            increment_pending_count(
                &mut self.state.pending_counts_by_channel,
                &request.channel_id,
            );
        }
        self.known_pending_request_channels
            .insert(request.id.clone(), request.channel_id.clone());
    }

    fn visible_requests_for_selected_channel(&self, requests: Vec<Request>) -> Vec<Request> {
        if let Some(channel_id) = self.state.selected_channel_id.as_deref() {
            visible_requests(
                requests
                    .into_iter()
                    .filter(|request| request.channel_id == channel_id)
                    .collect(),
            )
        } else {
            visible_requests(requests)
        }
    }

    fn ensure_selected_request(&mut self) {
        if self
            .state
            .selected_request_id
            .as_deref()
            .map(|id| self.state.requests.iter().any(|request| request.id == id))
            .unwrap_or(false)
        {
            return;
        }
        self.state.selected_request_id = self
            .state
            .requests
            .first()
            .map(|request| request.id.clone());
    }
}

fn request_is_stale(candidate: &Request, current: &Request) -> bool {
    current.updated_at > candidate.updated_at
        || (current.updated_at == candidate.updated_at
            && current.status != RequestStatus::Pending
            && candidate.status == RequestStatus::Pending)
}

fn pending_requests(requests: &[Request]) -> Vec<Request> {
    requests
        .iter()
        .filter(|request| request.status == RequestStatus::Pending)
        .cloned()
        .collect()
}

fn count_pending_by_channel(requests: &[Request]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for request in requests {
        increment_pending_count(&mut counts, &request.channel_id);
    }
    counts
}

fn pending_request_channels(requests: &[Request]) -> BTreeMap<String, String> {
    requests
        .iter()
        .map(|request| (request.id.clone(), request.channel_id.clone()))
        .collect()
}

fn increment_pending_count(counts: &mut BTreeMap<String, usize>, channel_id: &str) {
    *counts.entry(channel_id.to_string()).or_insert(0) += 1;
}

fn decrement_pending_count(counts: &mut BTreeMap<String, usize>, channel_id: &str) {
    if let Some(count) = counts.get_mut(channel_id) {
        *count = count.saturating_sub(1);
    }
    counts.retain(|_, count| *count > 0);
}

fn visible_requests(mut requests: Vec<Request>) -> Vec<Request> {
    requests.sort_by(|lhs, rhs| {
        request_status_rank(&lhs.status)
            .cmp(&request_status_rank(&rhs.status))
            .then_with(|| rhs.created_at.cmp(&lhs.created_at))
            .then_with(|| rhs.id.cmp(&lhs.id))
    });
    let mut handled = 0;
    requests
        .into_iter()
        .filter(|request| {
            if request.status == RequestStatus::Pending {
                return true;
            }
            handled += 1;
            handled <= HANDLED_REQUEST_DISPLAY_LIMIT
        })
        .collect()
}

fn request_status_rank(status: &RequestStatus) -> u8 {
    match status {
        RequestStatus::Pending => 0,
        RequestStatus::Resolved | RequestStatus::Expired | RequestStatus::Cancelled => 1,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::models::{DecisionResolution, Request};

    fn channel(id: &str, subscribed: bool) -> Channel {
        Channel {
            id: id.to_string(),
            name: id.to_string(),
            emoji: "🔔".to_string(),
            subscribed,
            created_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
        }
    }

    fn request(id: &str, channel_id: &str, status: RequestStatus) -> Request {
        Request {
            id: id.to_string(),
            request_id: id.to_string(),
            channel_id: channel_id.to_string(),
            recipients: Vec::new(),
            decision_resolution: DecisionResolution::Shared,
            title: id.to_string(),
            summary: String::new(),
            body_markdown: String::new(),
            fields: Vec::new(),
            links: Vec::new(),
            image_url: None,
            notification: Default::default(),
            dedupe_key: None,
            expires_at: None,
            status,
            created_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: None,
            options: Vec::new(),
            request_digest: None,
            signing: None,
        }
    }

    #[test]
    fn refresh_suppresses_initial_notification_snapshot() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".to_string());
        reducer.state.selected_channel_id = Some("default".to_string());
        let candidates = reducer.apply_refresh(
            None,
            Vec::new(),
            Vec::new(),
            vec![request("a", "default", RequestStatus::Pending)],
        );
        assert!(candidates.is_empty());
        let candidates = reducer.apply_refresh(
            None,
            Vec::new(),
            Vec::new(),
            vec![
                request("a", "default", RequestStatus::Pending),
                request("b", "default", RequestStatus::Pending),
            ],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "b");
    }

    #[test]
    fn refresh_returns_to_all_channels_when_selection_is_hidden() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".to_string());
        reducer.state.selected_channel_id = Some("hidden".to_string());

        reducer.apply_refresh(
            None,
            Vec::new(),
            vec![channel("hidden", false), channel("visible", true)],
            vec![request("a", "visible", RequestStatus::Pending)],
        );

        assert_eq!(reducer.state.selected_channel_id.as_deref(), None);
        assert_eq!(reducer.state.requests[0].channel_id, "visible");
    }

    #[test]
    fn request_update_reduces_pending_count_when_resolved() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".to_string());
        reducer.state.selected_channel_id = Some("default".to_string());
        let pending = request("a", "default", RequestStatus::Pending);
        reducer.apply_refresh(None, Vec::new(), Vec::new(), vec![pending.clone()]);
        assert_eq!(
            reducer.state.pending_counts_by_channel.get("default"),
            Some(&1)
        );

        let mut resolved = pending;
        resolved.status = RequestStatus::Resolved;
        reducer.apply_request_update(resolved);

        assert_eq!(reducer.state.pending_counts_by_channel.get("default"), None);
        assert_eq!(reducer.state.requests[0].status, RequestStatus::Resolved);
    }

    #[test]
    fn request_update_reduces_pending_count_when_cancelled() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".to_string());
        reducer.state.selected_channel_id = Some("default".to_string());
        let pending = request("a", "default", RequestStatus::Pending);
        reducer.apply_refresh(None, Vec::new(), Vec::new(), vec![pending.clone()]);

        let mut cancelled = pending;
        cancelled.status = RequestStatus::Cancelled;
        reducer.apply_request_update(cancelled);

        assert_eq!(reducer.state.pending_counts_by_channel.get("default"), None);
        assert_eq!(reducer.state.requests[0].status, RequestStatus::Cancelled);
    }

    #[test]
    fn request_update_moves_pending_count_between_channels() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".to_string());
        reducer.state.selected_channel_id = Some("alpha".to_string());
        let pending = request("a", "alpha", RequestStatus::Pending);
        reducer.apply_refresh(None, Vec::new(), Vec::new(), vec![pending.clone()]);

        let mut moved = pending;
        moved.channel_id = "beta".to_string();
        reducer.apply_request_update(moved);

        assert_eq!(reducer.state.pending_counts_by_channel.get("alpha"), None);
        assert_eq!(
            reducer.state.pending_counts_by_channel.get("beta"),
            Some(&1)
        );
    }

    #[test]
    fn visible_requests_keep_pending_and_limit_handled_requests() {
        let mut requests: Vec<_> = (0..=HANDLED_REQUEST_DISPLAY_LIMIT)
            .map(|index| {
                let id = format!("resolved-{index}");
                request(&id, "default", RequestStatus::Resolved)
            })
            .collect();
        requests.push(request("pending", "default", RequestStatus::Pending));

        let visible = visible_requests(requests);
        let handled_count = visible
            .iter()
            .filter(|request| request.status != RequestStatus::Pending)
            .count();

        assert!(visible.iter().any(|request| request.id == "pending"));
        assert_eq!(handled_count, HANDLED_REQUEST_DISPLAY_LIMIT);
    }
    #[test]
    fn every_terminal_envelope_updates_pending_state() {
        for (kind, status) in [
            ("resolved", RequestStatus::Resolved),
            ("expired", RequestStatus::Expired),
            ("cancelled", RequestStatus::Cancelled),
        ] {
            let mut reducer = StateReducer::new(Vec::new(), None, "default".into());
            reducer.apply_request_update(request("a", "default", RequestStatus::Pending));
            let candidates = reducer.apply_sync_envelope(SyncEnvelope {
                kind: kind.into(),
                at: Utc::now(),
                notification_delivery: None,
                payload: crate::models::SyncPayload {
                    request: Some(request("a", "default", status.clone())),
                    ..Default::default()
                },
            });
            assert_eq!(reducer.state.requests[0].status, status);
            assert!(reducer.state.pending_counts_by_channel.is_empty());
            assert!(candidates.is_empty());
        }
    }

    #[test]
    fn stale_created_event_cannot_undo_snapshot_resolution() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".into());
        reducer.apply_refresh(
            None,
            vec![],
            vec![],
            vec![request("a", "default", RequestStatus::Resolved)],
        );
        assert!(!reducer.apply_request_update(request("a", "default", RequestStatus::Pending)));
        assert_eq!(reducer.state.requests[0].status, RequestStatus::Resolved);
    }

    #[test]
    fn snapshot_does_not_overwrite_events_received_during_fetch() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".into());
        let revision = reducer.revision();
        reducer.apply_request_update(request("a", "default", RequestStatus::Resolved));
        reducer.apply_snapshot(
            None,
            vec![],
            vec![],
            vec![request("a", "default", RequestStatus::Pending)],
            revision,
        );
        assert_eq!(reducer.state.requests[0].status, RequestStatus::Resolved);
    }

    #[test]
    fn delayed_event_during_fetch_cannot_overwrite_newer_snapshot() {
        for seconds_later in [0, 1] {
            let mut reducer = StateReducer::new(Vec::new(), None, "default".into());
            let revision = reducer.revision();
            reducer.apply_request_update(request("a", "default", RequestStatus::Pending));
            let mut resolved = request("a", "default", RequestStatus::Resolved);
            resolved.updated_at += chrono::Duration::seconds(seconds_later);
            reducer.apply_snapshot(None, vec![], vec![], vec![resolved], revision);
            assert_eq!(reducer.state.requests[0].status, RequestStatus::Resolved);
            assert!(reducer.state.pending_counts_by_channel.is_empty());
        }
    }

    #[test]
    fn channel_selection_uses_the_complete_cached_inbox() {
        let mut reducer = StateReducer::new(Vec::new(), None, "default".into());
        reducer.apply_refresh(
            None,
            vec![],
            vec![channel("a", true), channel("b", true)],
            vec![
                request("one", "a", RequestStatus::Pending),
                request("two", "b", RequestStatus::Pending),
            ],
        );
        reducer.select_channel(Some("b".into()));
        assert_eq!(reducer.state.requests.len(), 1);
        assert_eq!(reducer.state.requests[0].id, "two");
        reducer.select_channel(None);
        assert_eq!(reducer.state.requests.len(), 2);
    }
}
