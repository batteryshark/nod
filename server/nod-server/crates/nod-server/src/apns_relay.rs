use std::{fs, sync::Arc, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use nod_apns_relay::{
    ApnsRelayRequest, AppleApnsProvider, DynApnsDelivery, NotificationContent,
    NotificationMetadata, NotificationTarget, RelayPolicy,
};
use reqwest::{Certificate, Client, Identity};
use url::Url;

use crate::{
    config::{ApnsDirectConfig, ApnsRelayConfig},
    models::{DecisionRequest, Device},
    push::{PushCategory, PushProvider, APPLE_APNS_PROVIDER_ID},
};

const APNS_RELAY_PUSH_PATH: &str = "/v1/notifications";

/// Remote push provider: forwards notifications to a standalone `nod-apns-relay`
/// over mTLS. Used for scale-out deployments where the relay (and the APNs
/// signing key) live in a separate process or host.
pub struct ApnsRelayProvider {
    client: Client,
    url: String,
    native_app_id: String,
}

impl ApnsRelayProvider {
    pub fn new(config: ApnsRelayConfig) -> anyhow::Result<Self> {
        let raw_url = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing APNs relay URL"))?;
        let url = Url::parse(raw_url)?;
        if url.scheme() != "https" {
            anyhow::bail!("APNs relay URL must use https");
        }
        let native_app_id = required_native_app_id(config.native_app_id.as_deref())?;
        Ok(Self {
            client: relay_client(&config)?,
            url: url.as_str().trim_end_matches('/').to_string(),
            native_app_id,
        })
    }
}

#[async_trait]
impl PushProvider for ApnsRelayProvider {
    fn id(&self) -> &str {
        APPLE_APNS_PROVIDER_ID
    }

    fn native_app_id(&self) -> Option<&str> {
        Some(&self.native_app_id)
    }

    async fn push_request(&self, device: &Device, request: &DecisionRequest) -> anyhow::Result<()> {
        let Some(relay_request) = build_relay_request(device, request) else {
            return Ok(());
        };
        let response = self
            .client
            .post(format!("{}{}", self.url, APNS_RELAY_PUSH_PATH))
            .json(&relay_request)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if !response.status().is_success() {
            let status = response.status();
            let failure = nod_apns_relay::error::bounded_error_json::<
                nod_apns_relay::error::DeliveryFailure,
            >(response)
            .await
            .unwrap_or(nod_apns_relay::error::DeliveryFailure {
                message: format!("APNs relay rejected push with {status}"),
                retryable: status.is_server_error() || status.as_u16() == 429,
                invalid_token: false,
            });
            return Err(failure.into());
        }
        tracing::info!(device_id = %device.id, request_id = %request.id, "push delivered to APNs relay");
        Ok(())
    }
}

/// In-process push provider: embeds the relay's Apple delivery and pushes to
/// APNs directly, with no HTTP hop or mTLS. Used when the server is configured
/// with local Apple credentials (co-located deployment). The same
/// [`RelayPolicy`] bundle-id pinning and field validation as the standalone
/// relay still apply before anything reaches Apple.
pub struct InProcessApnsProvider {
    delivery: DynApnsDelivery,
    policy: RelayPolicy,
    native_app_id: String,
}

impl InProcessApnsProvider {
    pub fn new(config: &ApnsDirectConfig) -> anyhow::Result<Self> {
        let apns = direct_apns_config(config)?;
        let native_app_id = apns.bundle_id.clone();
        let delivery: DynApnsDelivery = Arc::new(AppleApnsProvider::new(apns)?);
        Ok(Self::with_delivery(delivery, native_app_id))
    }

    fn with_delivery(delivery: DynApnsDelivery, native_app_id: String) -> Self {
        Self {
            policy: RelayPolicy::new(native_app_id.clone()),
            delivery,
            native_app_id,
        }
    }
}

#[async_trait]
impl PushProvider for InProcessApnsProvider {
    fn id(&self) -> &str {
        APPLE_APNS_PROVIDER_ID
    }

    fn native_app_id(&self) -> Option<&str> {
        Some(&self.native_app_id)
    }

    async fn push_request(&self, device: &Device, request: &DecisionRequest) -> anyhow::Result<()> {
        let Some(relay_request) = build_relay_request(device, request) else {
            return Ok(());
        };
        let notification = self.policy.sanitize(relay_request)?;
        self.delivery.send(&notification).await?;
        tracing::info!(device_id = %device.id, request_id = %request.id, "push delivered to APNs in-process");
        Ok(())
    }
}

/// Translate the server's local Apple credentials into the relay crate's
/// [`nod_apns_relay::ApnsConfig`]. The environment string is parsed by the relay
/// (the single source of truth), so an invalid value fails fast at startup.
fn direct_apns_config(config: &ApnsDirectConfig) -> anyhow::Result<nod_apns_relay::ApnsConfig> {
    let bundle_id = required_field(
        "notifications.apns_direct.bundle_id",
        config.bundle_id.as_deref(),
    )?;
    let team_id = required_field(
        "notifications.apns_direct.team_id",
        config.team_id.as_deref(),
    )?;
    let key_id = required_field("notifications.apns_direct.key_id", config.key_id.as_deref())?;
    let private_key_path = config
        .private_key_path
        .clone()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("notifications.apns_direct.private_key_path is required"))?;
    let environment = match config
        .environment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => nod_apns_relay::ApnsEnvironment::parse(value)?,
        None => nod_apns_relay::ApnsEnvironment::Production,
    };
    Ok(nod_apns_relay::ApnsConfig {
        bundle_id,
        environment,
        credentials: nod_apns_relay::ApnsCredentials {
            team_id,
            key_id,
            private_key_path,
        },
    })
}

fn required_field(field: &str, value: Option<&str>) -> anyhow::Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{field} is required"))
}

/// Build the relay wire request from a device + decision request. Shared by the
/// remote and in-process providers so there is exactly one mapping from Nod's
/// domain types to the relay contract. Returns `None` when the device is not a
/// configured APNs target.
fn build_relay_request(device: &Device, request: &DecisionRequest) -> Option<ApnsRelayRequest> {
    device
        .push_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| *provider == APPLE_APNS_PROVIDER_ID)?;
    let token = device.push_token.as_deref()?.trim();
    if token.is_empty() {
        return None;
    }
    let mut wire = request.to_wire();
    if device.notification_preferences.hide_content {
        wire.notification = nod_proto::RequestNotification {
            redact: true,
            title: None,
            body: None,
        };
    }
    let preview = nod_proto::notification_preview(&wire);
    Some(ApnsRelayRequest {
        target: NotificationTarget {
            platform: device.platform.as_str().to_string(),
            native_app_id: device.native_app_id.as_ref()?.to_string(),
            token: token.to_string(),
        },
        notification: NotificationContent {
            title: preview.title,
            body: preview.body,
            sound: device.notification_sound.clone(),
            thread_id: request.channel_id.clone(),
            category: PushCategory::for_request(request).as_str().to_string(),
        },
        metadata: NotificationMetadata {
            request_id: request.id.clone(),
            channel_id: request.channel_id.clone(),
            device_id: Some(device.id.clone()),
        },
    })
}

fn relay_client(config: &ApnsRelayConfig) -> anyhow::Result<Client> {
    let tls = &config.tls;
    let client_cert_path = tls
        .client_cert_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing APNs relay client certificate path"))?;
    let client_key_path = tls
        .client_key_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing APNs relay client key path"))?;
    let ca_cert_path = tls
        .ca_cert_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing APNs relay CA certificate path"))?;

    let mut identity_pem = fs::read(client_cert_path).with_context(|| {
        format!(
            "failed to read APNs relay client certificate {}",
            client_cert_path.display()
        )
    })?;
    identity_pem.extend(fs::read(client_key_path).with_context(|| {
        format!(
            "failed to read APNs relay client key {}",
            client_key_path.display()
        )
    })?);
    let identity = Identity::from_pem(&identity_pem)?;
    let ca_pem = fs::read(ca_cert_path).with_context(|| {
        format!(
            "failed to read APNs relay CA certificate {}",
            ca_cert_path.display()
        )
    })?;
    let certificates = Certificate::from_pem_bundle(&ca_pem)?;
    if certificates.is_empty() {
        anyhow::bail!("APNs relay CA bundle did not contain certificates");
    }

    let mut builder = Client::builder()
        .https_only(true)
        .identity(identity)
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    for certificate in certificates {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(builder.build()?)
}

fn required_native_app_id(value: Option<&str>) -> anyhow::Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing APNs relay native app id"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Utc;
    use nod_apns_relay::{ApnsDelivery, RelayNotification};

    use crate::config::{ApnsRelayConfig, ApnsRelayTlsConfig};
    use crate::models::{DecisionResolution, DevicePlatform, RequestStatus};

    use super::*;

    fn apns_device() -> Device {
        let now = Utc::now();
        Device {
            id: "device-1".to_string(),
            user_id: "owner".to_string(),
            name: "Phone".to_string(),
            platform: DevicePlatform::Ios,
            native_app_id: Some("com.example.NodTests".to_string()),
            push_provider: Some("apple_apns".to_string()),
            push_token: Some("push-token".to_string()),
            signing_key_id: None,
            signing_key_algorithm: None,
            signing_public_key: None,
            notification_sound: "default".to_string(),
            notification_preferences: Default::default(),
            last_seen_at: now,
            created_at: now,
        }
    }

    fn apns_request() -> DecisionRequest {
        let now = Utc::now();
        DecisionRequest {
            id: "request-1".to_string(),
            channel_id: "default".to_string(),
            recipients: vec!["owner".to_string()],
            decision_resolution: DecisionResolution::Shared,
            title: "Deploy".to_string(),
            summary: "Production deploy is waiting".to_string(),
            body_markdown: "secret details".to_string(),
            fields: vec![],
            links: vec![],
            image_url: None,
            notification: Default::default(),
            dedupe_key: None,
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            resolved_at: None,
            decision: None,
            user_decisions: vec![],
            callback_url: None,
            options: vec![],
            private_recipients: false,
            signing: None,
        }
    }

    #[derive(Clone, Default)]
    struct RecordingDelivery {
        sent: Arc<Mutex<Vec<RelayNotification>>>,
    }

    #[async_trait]
    impl ApnsDelivery for RecordingDelivery {
        async fn send(&self, notification: &RelayNotification) -> anyhow::Result<()> {
            self.sent.lock().unwrap().push(notification.clone());
            Ok(())
        }
    }

    #[test]
    fn build_relay_request_serializes_apns_relay_contract() {
        let request = build_relay_request(&apns_device(), &apns_request()).unwrap();
        let value = serde_json::to_value(request).unwrap();

        assert!(value.get("provider").is_none());
        assert_eq!(value["target"]["platform"], "ios");
        assert_eq!(value["target"]["native_app_id"], "com.example.NodTests");
        assert_eq!(value["target"]["token"], "push-token");
        assert_eq!(value["notification"]["title"], "Deploy");
        assert_eq!(
            value["notification"]["body"],
            "Production deploy is waiting"
        );
        assert_eq!(value["notification"]["category"], "NOD_DEFAULT");
        assert_eq!(value["metadata"]["request_id"], "request-1");
        assert!(value["body_markdown"].is_null());
    }

    #[test]
    fn build_relay_request_uses_redacted_notification_text() {
        let mut request = apns_request();
        request.title = "Secret deploy".to_string();
        request.summary = "Sensitive production details".to_string();
        request.notification = crate::models::RequestNotification {
            redact: true,
            title: Some("Nod".to_string()),
            body: Some("Open Nod to review.".to_string()),
        };

        let relay_request = build_relay_request(&apns_device(), &request).unwrap();
        assert_eq!(relay_request.notification.title, "Nod");
        assert_eq!(relay_request.notification.body, "Open Nod to review.");

        request.notification.title = None;
        request.notification.body = None;
        let relay_request = build_relay_request(&apns_device(), &request).unwrap();
        assert_eq!(relay_request.notification.title, "Nod");
        assert_eq!(
            relay_request.notification.body,
            "Open Nod to review this request."
        );
    }

    #[tokio::test]
    async fn in_process_provider_delivers_sanitized_notification() {
        let recorder = RecordingDelivery::default();
        let provider = InProcessApnsProvider::with_delivery(
            Arc::new(recorder.clone()),
            "com.example.NodTests".to_string(),
        );

        provider
            .push_request(&apns_device(), &apns_request())
            .await
            .unwrap();

        let sent = recorder.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].target.token, "push-token");
        assert_eq!(sent[0].notification.title, "Deploy");
        assert_eq!(sent[0].metadata.channel_id, "default");
    }

    #[tokio::test]
    async fn in_process_provider_enforces_bundle_id_pinning() {
        let recorder = RecordingDelivery::default();
        let provider = InProcessApnsProvider::with_delivery(
            Arc::new(recorder.clone()),
            "com.example.Other".to_string(),
        );

        let err = provider
            .push_request(&apns_device(), &apns_request())
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("does not match"), "{err}");
        assert!(recorder.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn apns_relay_provider_loads_mtls_identity() {
        let config = ApnsRelayConfig {
            url: Some("https://relay.example.com".to_string()),
            native_app_id: Some("com.example.NodTests".to_string()),
            tls: ApnsRelayTlsConfig {
                client_cert_path: Some("tests/fixtures/relay-tls/client.crt".into()),
                client_key_path: Some("tests/fixtures/relay-tls/client.key".into()),
                ca_cert_path: Some("tests/fixtures/relay-tls/server-ca.crt".into()),
            },
        };

        ApnsRelayProvider::new(config).unwrap();
    }
    #[test]
    fn title_only_request_has_a_safe_body_and_custom_options_use_open_fallback() {
        let mut request = apns_request();
        request.summary.clear();
        request.body_markdown.clear();
        request.options=vec![serde_json::from_value(serde_json::json!({"id":"approve","kind":"approve","label":"Delete production","destructive":true})).unwrap()];
        let relay = build_relay_request(&apns_device(), &request).unwrap();
        assert_eq!(relay.notification.body, "Open Nod to review this request.");
        assert_eq!(relay.notification.category, "NOD_DEFAULT");
        assert_eq!(relay.metadata.device_id.as_deref(), Some("device-1"));
    }
    #[test]
    fn device_hide_content_overrides_issuer_preview_without_changing_request() {
        let mut device = apns_device();
        device.notification_preferences.hide_content = true;
        let mut request = apns_request();
        request.notification.title = Some("Issuer preview secret".to_string());
        request.notification.body = Some("More secret content".to_string());
        let relay = build_relay_request(&device, &request).unwrap();
        assert_eq!(relay.notification.title, "Nod");
        assert_eq!(relay.notification.body, "Open Nod to review this request.");
        assert_eq!(request.title, "Deploy");
        assert_eq!(
            request.notification.title.as_deref(),
            Some("Issuer preview secret")
        );
    }
}
