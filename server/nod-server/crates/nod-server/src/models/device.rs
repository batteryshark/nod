use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    default_signature_algorithm, Channel, DeviceAttestationSummary, NotificationDelivery, User,
};

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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Macos => "macos",
            Self::Watchos => "watchos",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Unknown => "unknown",
        }
    }
}

impl From<&str> for DevicePlatform {
    fn from(value: &str) -> Self {
        match value {
            "ios" => Self::Ios,
            "macos" => Self::Macos,
            "watchos" => Self::Watchos,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub native_app_id: Option<String>,
    pub push_provider: Option<String>,
    pub push_token: Option<String>,
    pub signing_key_id: Option<String>,
    pub signing_key_algorithm: Option<String>,
    pub signing_public_key: Option<String>,
    pub notification_sound: String,
    pub notification_preferences: nod_proto::DeviceNotificationPreferences,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminDevice {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub native_app_id: Option<String>,
    pub push_provider: Option<String>,
    pub has_push_token: bool,
    pub has_signing_key: bool,
    pub notification_sound: String,
    pub notification_preferences: nod_proto::DeviceNotificationPreferences,
    pub attestation: Option<DeviceAttestationSummary>,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub subscriptions: Vec<Channel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserDevice {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub native_app_id: Option<String>,
    pub push_provider: Option<String>,
    pub has_push_token: bool,
    pub has_signing_key: bool,
    pub notification_sound: String,
    pub notification_preferences: nod_proto::DeviceNotificationPreferences,
    pub attestation: Option<DeviceAttestationSummary>,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub is_current: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnrollDeviceRequest {
    pub code: String,
    pub device_name: String,
    pub platform: DevicePlatform,
    #[serde(default)]
    pub native_app_id: Option<String>,
    #[serde(default)]
    pub push_provider: Option<String>,
    #[serde(default)]
    pub push_token: Option<String>,
    #[serde(default)]
    pub signing_key: Option<DeviceSigningKeyRequest>,
    #[serde(default)]
    pub attestation: Option<DeviceAttestationRequest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceSigningKeyRequest {
    pub key_id: String,
    #[serde(default = "default_signature_algorithm")]
    pub algorithm: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAttestationRequest {
    pub provider: String,
    pub key_id: String,
    pub attestation_object: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrollDeviceResponse {
    pub device_id: String,
    pub user_id: String,
    pub user_name: String,
    pub token: String,
    pub notification_delivery: NotificationDelivery,
    pub channels: Vec<Channel>,
    pub devices: Vec<UserDevice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentUserResponse {
    pub user: User,
    pub current_device: UserDevice,
    pub notification_delivery: NotificationDelivery,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserDeviceRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePushTokenRequest {
    pub provider: String,
    pub token: String,
    pub native_app_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDevicePreferencesRequest {
    #[serde(default)]
    pub notification_sound: Option<String>,
}
