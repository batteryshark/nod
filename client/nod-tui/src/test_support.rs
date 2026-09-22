use std::collections::BTreeMap;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use nod_client_core::{
    models::{
        ClientState, DecisionResolution, DevicePlatform, NotificationDeliveryMode, Request,
        RequestStatus, ServerProfile, UserDevice,
    },
    ChannelParams, EnrollParams, NotificationPreferenceParams, RenameDeviceParams,
    RevokeDeviceParams, SelectRequestParams, SelectServerParams, SetSubscriptionParams,
    SubmitOptionParams,
};

use crate::runtime_bridge::RuntimePort;

pub fn client_state() -> ClientState {
    ClientState {
        servers: vec![ServerProfile {
            id: "local".to_string(),
            credential_id: None,
            name: "Local".to_string(),
            base_url_string: "http://localhost:8767".to_string(),
            device_name: "terminal".to_string(),
            device_id: Some("device".to_string()),
            user_id: Some("owner".to_string()),
            user_name: Some("Owner".to_string()),
        }],
        selected_server_id: Some("local".to_string()),
        current_user: None,
        devices: Vec::new(),
        channels: vec![nod_client_core::models::Channel {
            id: "default".to_string(),
            name: "Default".to_string(),
            emoji: "🔔".to_string(),
            subscribed: true,
            created_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
        }],
        pending_counts_by_channel: BTreeMap::from([("default".to_string(), 1)]),
        requests: vec![request("deploy", "default")],
        selected_channel_id: Some("default".to_string()),
        selected_request_id: Some("deploy".to_string()),
        notification_sound: "default".to_string(),
        notification_delivery_mode: NotificationDeliveryMode::Websocket,
        is_registered: true,
        is_sync_connected: false,
        sync_phase: Default::default(),
        last_synced_at: None,
        last_error: None,
    }
}

pub fn request(id: &str, channel_id: &str) -> Request {
    request_with_status(id, channel_id, RequestStatus::Pending)
}

pub fn request_with_status(id: &str, channel_id: &str, status: RequestStatus) -> Request {
    Request {
        id: id.to_string(),
        request_id: id.to_string(),
        channel_id: channel_id.to_string(),
        recipients: Vec::new(),
        decision_resolution: DecisionResolution::Shared,
        title: id.to_string(),
        summary: format!("{id} summary"),
        body_markdown: format!("{id} body"),
        fields: Vec::new(),
        links: Vec::new(),
        image_url: None,
        notification: Default::default(),
        dedupe_key: None,
        expires_at: None,
        status,
        created_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
        resolved_at: None,
        decision: None,
        decisions: Vec::new(),
        callback_url: None,
        options: Vec::new(),
        request_digest: Some("digest".to_string()),
        signing: None,
    }
}

/// In-memory [`RuntimePort`] for tests: records the methods called and serves
/// a configurable state/request instead of touching the network.
#[derive(Debug)]
pub struct FakeRuntime {
    pub calls: Vec<&'static str>,
    pub fail_submit: bool,
    pub fail_connect: bool,
    /// Returned by every state-yielding method (enroll, refresh, selects, …).
    pub state: ClientState,
    /// Returned by `submit_option` when `fail_submit` is false.
    pub submit_result: Request,
}

impl Default for FakeRuntime {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            fail_submit: false,
            fail_connect: false,
            state: client_state(),
            submit_result: request("deploy", "default"),
        }
    }
}

#[async_trait]
impl RuntimePort for FakeRuntime {
    async fn enroll(&mut self, _params: EnrollParams) -> anyhow::Result<ClientState> {
        self.calls.push("enroll");
        Ok(self.state.clone())
    }

    async fn refresh(&mut self) -> anyhow::Result<ClientState> {
        self.calls.push("refresh");
        Ok(self.state.clone())
    }

    async fn connect_sync(&mut self) -> anyhow::Result<()> {
        self.calls.push("connect_sync");
        if self.fail_connect {
            return Err(anyhow!("connection unavailable"));
        }
        Ok(())
    }

    async fn select_server(&mut self, _params: SelectServerParams) -> anyhow::Result<ClientState> {
        self.calls.push("select_server");
        Ok(self.state.clone())
    }

    async fn forget_server(&mut self, _params: SelectServerParams) -> anyhow::Result<ClientState> {
        self.calls.push("forget_server");
        Ok(self.state.clone())
    }

    async fn select_channel(&mut self, _params: ChannelParams) -> anyhow::Result<ClientState> {
        self.calls.push("select_channel");
        Ok(self.state.clone())
    }

    async fn query_history(
        &mut self,
        _params: nod_client_core::QueryHistoryParams,
    ) -> anyhow::Result<nod_client_core::models::RequestsResponse> {
        Ok(nod_client_core::models::RequestsResponse {
            requests: self.state.requests.clone(),
            next_cursor: None,
        })
    }

    async fn select_all_channels(&mut self) -> anyhow::Result<ClientState> {
        self.calls.push("select_all_channels");
        self.state.selected_channel_id = None;
        Ok(self.state.clone())
    }

    async fn select_request(
        &mut self,
        _params: SelectRequestParams,
    ) -> anyhow::Result<ClientState> {
        self.calls.push("select_request");
        Ok(self.state.clone())
    }

    async fn submit_option(&mut self, _params: SubmitOptionParams) -> anyhow::Result<Request> {
        self.calls.push("submit_option");
        if self.fail_submit {
            return Err(anyhow!("stale request"));
        }
        Ok(self.submit_result.clone())
    }

    async fn clear_channel(&mut self, _params: ChannelParams) -> anyhow::Result<ClientState> {
        self.calls.push("clear_channel");
        Ok(self.state.clone())
    }

    async fn set_subscription(
        &mut self,
        _params: SetSubscriptionParams,
    ) -> anyhow::Result<ClientState> {
        self.calls.push("set_subscription");
        Ok(self.state.clone())
    }

    async fn set_notification_preference(
        &mut self,
        _params: NotificationPreferenceParams,
    ) -> anyhow::Result<ClientState> {
        self.calls.push("set_notification_preference");
        Ok(self.state.clone())
    }

    async fn list_devices(&mut self) -> anyhow::Result<Vec<UserDevice>> {
        self.calls.push("list_devices");
        Ok(vec![user_device("phone")])
    }

    async fn rename_device(&mut self, _params: RenameDeviceParams) -> anyhow::Result<UserDevice> {
        self.calls.push("rename_device");
        Ok(user_device("renamed"))
    }

    async fn revoke_device(&mut self, _params: RevokeDeviceParams) -> anyhow::Result<ClientState> {
        self.calls.push("revoke_device");
        Ok(self.state.clone())
    }
}

pub fn user_device(name: &str) -> UserDevice {
    UserDevice {
        id: name.to_string(),
        user_id: "owner".to_string(),
        name: name.to_string(),
        platform: DevicePlatform::Linux,
        native_app_id: None,
        push_provider: None,
        has_push_token: false,
        has_signing_key: false,
        attestation: None,
        notification_sound: "default".to_string(),
        notification_preferences: Default::default(),
        last_seen_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
        is_current: false,
    }
}
