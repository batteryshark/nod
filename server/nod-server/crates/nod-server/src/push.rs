use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    apns_relay,
    config::NotificationsConfig,
    models::{DecisionRequest, Device, NotificationDelivery, NotificationDeliveryMode},
};

pub const APPLE_APNS_PROVIDER_ID: &str = "apple_apns";

#[async_trait]
pub trait PushProvider: Send + Sync {
    fn id(&self) -> &str;

    fn native_app_id(&self) -> Option<&str> {
        None
    }

    async fn push_request(&self, device: &Device, request: &DecisionRequest) -> anyhow::Result<()>;
}

pub type DynPushProvider = Arc<dyn PushProvider>;

#[derive(Clone, Default)]
pub struct PushRegistry {
    providers_by_route: Arc<HashMap<PushRouteKey, DynPushProvider>>,
}

impl PushRegistry {
    pub fn new(providers: Vec<DynPushProvider>) -> Self {
        let mut providers_by_route = HashMap::new();
        for provider in providers {
            let route = PushRouteKey::new(provider.id(), provider.native_app_id());
            providers_by_route.insert(route, provider);
        }
        Self {
            providers_by_route: Arc::new(providers_by_route),
        }
    }

    pub async fn push_request(
        &self,
        device: &Device,
        request: &DecisionRequest,
    ) -> anyhow::Result<()> {
        let Some(route) = PushRouteKey::from_device(device) else {
            tracing::debug!(device_id = %device.id, request_id = %request.id, "push skipped because device has no provider");
            return Ok(());
        };
        let Some(provider) = self.providers_by_route.get(&route) else {
            tracing::debug!(
                device_id = %device.id,
                request_id = %request.id,
                provider_id = %route.provider_id,
                native_app_id = route.native_app_id.as_deref(),
                "push skipped because provider route is not configured"
            );
            anyhow::bail!("push provider route is not configured");
        };
        provider.push_request(device, request).await
    }

    pub fn has_route(&self, device: &Device) -> bool {
        if device.push_provider.as_deref() == Some(APPLE_APNS_PROVIDER_ID)
            && !matches!(
                device.platform,
                crate::models::DevicePlatform::Ios | crate::models::DevicePlatform::Watchos
            )
        {
            return false;
        }
        PushRouteKey::from_device(device)
            .is_some_and(|route| self.providers_by_route.contains_key(&route))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PushRouteKey {
    provider_id: String,
    native_app_id: Option<String>,
}

impl PushRouteKey {
    fn new(provider_id: &str, native_app_id: Option<&str>) -> Self {
        Self {
            provider_id: provider_id.trim().to_string(),
            native_app_id: native_app_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        }
    }

    fn from_device(device: &Device) -> Option<Self> {
        let provider_id = device
            .push_provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        Some(Self::new(provider_id, device.native_app_id.as_deref()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushCategory {
    #[serde(rename = "NOD_DEFAULT")]
    Default,
    #[serde(rename = "NOD_APPROVAL")]
    Approval,
    #[serde(rename = "NOD_APPROVAL_TEXT")]
    ApprovalText,
}

impl PushCategory {
    pub fn for_request(request: &DecisionRequest) -> Self {
        let standard = [
            ("approve", "Approve", crate::models::OptionKind::Approve),
            ("reject", "Reject", crate::models::OptionKind::Reject),
        ];
        let matches = request.options.len() == standard.len()
            && standard.iter().all(|(id, label, kind)| {
                request.options.iter().any(|option| {
                    option.id == *id
                        && option.label == *label
                        && option.kind == *kind
                        && !option.requires_text
                        && !option.foreground
                        && !(option.destructive && *id == "approve")
                })
            });
        if matches {
            Self::Approval
        } else {
            Self::Default
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "NOD_DEFAULT",
            Self::Approval => "NOD_APPROVAL",
            Self::ApprovalText => "NOD_APPROVAL_TEXT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushRoute {
    /// Forward to a standalone `nod-apns-relay` over mTLS (scale-out).
    ApnsRelay,
    /// Deliver to Apple in-process, embedding the relay (co-located, no mTLS).
    ApnsDirect,
}

impl PushRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApnsRelay => "apns_relay",
            Self::ApnsDirect => "apns_direct",
        }
    }
}

#[derive(Clone)]
pub struct BuiltPushRegistry {
    pub registry: PushRegistry,
    pub route: Option<PushRoute>,
}

pub fn notification_delivery_for_route(route: Option<PushRoute>) -> NotificationDelivery {
    NotificationDelivery {
        mode: if route.is_some() {
            NotificationDeliveryMode::Push
        } else {
            NotificationDeliveryMode::Websocket
        },
    }
}

/// Decide the active push route from configuration. In-process direct delivery
/// and the remote relay are mutually exclusive; configuring both is a hard error
/// rather than a silent precedence rule.
pub fn configured_push_route(config: &NotificationsConfig) -> anyhow::Result<Option<PushRoute>> {
    match (
        config.apns_direct.enabled(),
        config.apns_relay.client_enabled(),
    ) {
        (true, true) => anyhow::bail!(
            "configure either in-process APNs (notifications.apns_direct / NOD_APNS_DIRECT_*) \
             or the remote relay (notifications.apns_relay / NOD_APNS_RELAY_*), not both"
        ),
        (true, false) => Ok(Some(PushRoute::ApnsDirect)),
        (false, true) => Ok(Some(PushRoute::ApnsRelay)),
        (false, false) => Ok(None),
    }
}

pub fn build_push_registry(config: &NotificationsConfig) -> anyhow::Result<BuiltPushRegistry> {
    let route = configured_push_route(config)?;
    let registry = match route {
        Some(PushRoute::ApnsRelay) => {
            let provider = apns_relay::ApnsRelayProvider::new(config.apns_relay.clone())?;
            PushRegistry::new(vec![Arc::new(provider)])
        }
        Some(PushRoute::ApnsDirect) => {
            let provider = apns_relay::InProcessApnsProvider::new(&config.apns_direct)?;
            PushRegistry::new(vec![Arc::new(provider)])
        }
        None => PushRegistry::default(),
    };
    Ok(BuiltPushRegistry { registry, route })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use crate::config::{
        ApnsDirectConfig, ApnsRelayConfig, ApnsRelayTlsConfig, NotificationsConfig,
    };

    use super::*;

    struct NoopProvider {
        native_app_id: Option<&'static str>,
    }

    #[async_trait]
    impl PushProvider for NoopProvider {
        fn id(&self) -> &str {
            APPLE_APNS_PROVIDER_ID
        }

        fn native_app_id(&self) -> Option<&str> {
            self.native_app_id
        }

        async fn push_request(
            &self,
            _device: &Device,
            _request: &DecisionRequest,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn registry_routes_are_scoped_by_native_app_id() {
        let registry = PushRegistry::new(vec![Arc::new(NoopProvider {
            native_app_id: Some("com.example.NodTests"),
        })]);

        assert!(registry.providers_by_route.contains_key(&PushRouteKey::new(
            APPLE_APNS_PROVIDER_ID,
            Some("com.example.NodTests")
        )));
        assert!(!registry
            .providers_by_route
            .contains_key(&PushRouteKey::new(APPLE_APNS_PROVIDER_ID, None)));
    }

    fn relay_config() -> ApnsRelayConfig {
        ApnsRelayConfig {
            url: Some("https://relay.example.com".to_string()),
            native_app_id: Some("com.example.NodTests".to_string()),
            tls: ApnsRelayTlsConfig {
                client_cert_path: Some("tests/fixtures/relay-tls/client.crt".into()),
                client_key_path: Some("tests/fixtures/relay-tls/client.key".into()),
                ca_cert_path: Some("tests/fixtures/relay-tls/server-ca.crt".into()),
            },
        }
    }

    fn direct_config() -> ApnsDirectConfig {
        ApnsDirectConfig {
            bundle_id: Some("com.example.NodTests".to_string()),
            team_id: Some("TEAMID".to_string()),
            key_id: Some("KEYID".to_string()),
            private_key_path: Some("tests/fixtures/mtls/apns-auth-key.p8".into()),
            environment: Some("sandbox".to_string()),
        }
    }

    #[test]
    fn apns_relay_route_selected_for_relay_config() {
        let config = NotificationsConfig {
            apns_relay: relay_config(),
            ..Default::default()
        };
        assert_eq!(
            configured_push_route(&config).unwrap(),
            Some(PushRoute::ApnsRelay)
        );
    }

    #[test]
    fn apns_direct_route_selected_for_direct_config() {
        let config = NotificationsConfig {
            apns_direct: direct_config(),
            ..Default::default()
        };
        assert_eq!(
            configured_push_route(&config).unwrap(),
            Some(PushRoute::ApnsDirect)
        );
    }

    #[test]
    fn configuring_both_routes_is_rejected() {
        let config = NotificationsConfig {
            apns_direct: direct_config(),
            apns_relay: relay_config(),
        };
        let err = configured_push_route(&config).unwrap_err().to_string();
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn websocket_selected_without_configured_provider() {
        assert_eq!(
            configured_push_route(&NotificationsConfig::default()).unwrap(),
            None
        );
    }

    #[test]
    fn build_fails_when_apns_relay_cert_files_are_missing() {
        let config = NotificationsConfig {
            apns_relay: ApnsRelayConfig {
                url: Some("https://relay.example.com".to_string()),
                native_app_id: Some("com.example.NodTests".to_string()),
                tls: ApnsRelayTlsConfig {
                    client_cert_path: Some("missing/client.crt".into()),
                    client_key_path: Some("missing/client.key".into()),
                    ca_cert_path: Some("missing/ca.crt".into()),
                },
            },
            ..Default::default()
        };

        let err = build_push_registry(&config).err().unwrap().to_string();

        assert!(err.contains("missing/client.crt"), "{err}");
    }

    #[test]
    fn build_reports_effective_push_for_valid_apns_relay() {
        let config = NotificationsConfig {
            apns_relay: relay_config(),
            ..Default::default()
        };
        let built = build_push_registry(&config).unwrap();
        assert_eq!(built.route, Some(PushRoute::ApnsRelay));
        assert_eq!(
            notification_delivery_for_route(built.route).mode,
            NotificationDeliveryMode::Push
        );
    }
}
