use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

// Wire types shared with the server live in nod-proto — the single channel of
// truth. Re-exported here so the rest of the client keeps using `models::*`.
pub use nod_proto::{
    CardField as Field, CardLink as Link, Decision, DecisionResolution, DecisionSignature,
    DeviceAttestationStatus, DeviceAttestationSummary, DeviceNotificationPreferences,
    NotificationDelivery, NotificationDeliveryMode, OptionKind, Request, RequestNotification,
    RequestOption, RequestStatus, UserDecision,
};

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Ios,
    Macos,
    Watchos,
    Windows,
    Linux,
    Unknown,
}

impl DevicePlatform {
    pub fn current_desktop() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Unknown
        }
    }
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub emoji: String,
    #[serde(default = "default_true")]
    pub subscribed: bool,
    #[typeshare(serialized_as = "String")]
    pub created_at: DateTime<Utc>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerProfile {
    pub id: String,
    /// Existing credentials keep their account name when URL-derived IDs migrate.
    #[serde(default)]
    pub credential_id: Option<String>,
    pub name: String,
    pub base_url_string: String,
    pub device_name: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_name: Option<String>,
}

impl ServerProfile {
    pub fn credential_id(&self) -> &str {
        self.credential_id.as_deref().unwrap_or(&self.id)
    }
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    #[typeshare(serialized_as = "String")]
    pub created_at: DateTime<Utc>,
    #[typeshare(serialized_as = "String")]
    pub updated_at: DateTime<Utc>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDevice {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub platform: DevicePlatform,
    #[serde(default)]
    pub native_app_id: Option<String>,
    pub push_provider: Option<String>,
    pub has_push_token: bool,
    #[serde(default)]
    pub has_signing_key: bool,
    #[serde(default)]
    pub attestation: Option<DeviceAttestationSummary>,
    pub notification_sound: String,
    #[serde(default)]
    pub notification_preferences: DeviceNotificationPreferences,
    #[typeshare(serialized_as = "String")]
    pub last_seen_at: DateTime<Utc>,
    #[typeshare(serialized_as = "String")]
    pub created_at: DateTime<Utc>,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceSigningKey {
    pub key_id: String,
    #[serde(default = "default_decision_signature_algorithm")]
    pub algorithm: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEnvelope {
    pub kind: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub notification_delivery: Option<NotificationDelivery>,
    #[serde(default)]
    pub payload: SyncPayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncPayload {
    #[serde(default)]
    pub request: Option<Request>,
    #[serde(default)]
    pub channel: Option<Channel>,
    #[serde(default)]
    pub notification_delivery: Option<NotificationDelivery>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    #[default]
    Offline,
    Connecting,
    Reconciling,
    Current,
    Revoked,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientState {
    pub servers: Vec<ServerProfile>,
    pub selected_server_id: Option<String>,
    pub current_user: Option<User>,
    pub devices: Vec<UserDevice>,
    pub channels: Vec<Channel>,
    #[typeshare(serialized_as = "HashMap<String, u32>")]
    pub pending_counts_by_channel: BTreeMap<String, usize>,
    pub requests: Vec<Request>,
    pub selected_channel_id: Option<String>,
    pub selected_request_id: Option<String>,
    pub notification_sound: String,
    #[serde(default)]
    pub notification_delivery_mode: NotificationDeliveryMode,
    pub is_registered: bool,
    pub is_sync_connected: bool,
    #[serde(default)]
    pub sync_phase: SyncPhase,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnrollDeviceResponse {
    pub device_id: String,
    pub token: String,
    pub channels: Vec<Channel>,
    pub user_id: String,
    pub user_name: String,
    #[serde(default)]
    pub devices: Vec<UserDevice>,
    #[serde(default)]
    pub notification_delivery: NotificationDelivery,
}

#[derive(Debug, Deserialize)]
pub struct CurrentUserResponse {
    pub user: User,
    pub current_device: UserDevice,
    #[serde(default)]
    pub notification_delivery: NotificationDelivery,
}

#[derive(Debug, Deserialize)]
pub struct UserDevicesResponse {
    pub devices: Vec<UserDevice>,
}

#[derive(Debug, Deserialize)]
pub struct UserDeviceResponse {
    pub device: UserDevice,
}

#[derive(Debug, Deserialize)]
pub struct ChannelsResponse {
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestsResponse {
    pub requests: Vec<Request>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestResponse {
    pub request: Request,
}

fn default_true() -> bool {
    true
}

fn default_decision_signature_algorithm() -> String {
    crate::signing::DECISION_SIGNING_ALGORITHM.to_string()
}
