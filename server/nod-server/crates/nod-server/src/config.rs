use std::{fmt, path::PathBuf};

use anyhow::bail;

mod environment;
mod file;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub retention_days: i64,
    /// None preserves trusted-issuer callbacks; Some([]) disables them.
    pub callback_allowed_origins: Option<Vec<String>>,
    pub notifications: NotificationsConfig,
    pub device_attestation: DeviceAttestationConfig,
    secrets: ServerSecrets,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let mut config = match environment::optional_env("NOD_CONFIG") {
            Some(path) => file::load_server_config(path)?,
            None => Self::without_secrets(),
        };
        environment::apply_server_env(&mut config)?;
        config.validate()?;
        Ok(config)
    }

    pub fn with_admin_token(admin_token: impl Into<String>) -> Self {
        let mut config = Self::without_secrets();
        config.secrets.set_admin_token(admin_token);
        config
    }

    pub fn admin_token(&self) -> &str {
        self.secrets.admin_token()
    }

    fn without_secrets() -> Self {
        Self {
            bind: default_bind(),
            database_url: default_database_url(),
            data_dir: default_data_dir(),
            retention_days: default_retention_days(),
            callback_allowed_origins: None,
            notifications: NotificationsConfig::default(),
            device_attestation: DeviceAttestationConfig::default(),
            secrets: ServerSecrets::empty(),
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.admin_token().trim().is_empty() {
            bail!("NOD_ADMIN_TOKEN or NOD_ADMIN_TOKEN_FILE is required");
        }
        if self.retention_days < 1 {
            bail!("retention_days must be at least 1");
        }
        if let Some(origins) = &self.callback_allowed_origins {
            for origin in origins {
                let url = url::Url::parse(origin)?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || url.path() != "/"
                {
                    bail!("callback_allowed_origins must contain only HTTP(S) origins");
                }
            }
        }
        self.notifications.apns_direct.validate()?;
        self.notifications.apns_relay.validate()?;
        self.device_attestation.apple_app_attest.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct DeviceAttestationConfig {
    pub apple_app_attest: AppleAppAttestConfig,
}

#[derive(Debug, Clone)]
pub struct AppleAppAttestConfig {
    pub mode: DeviceAttestationMode,
    pub team_id: Option<String>,
    pub bundle_ids: Vec<String>,
    pub environment: AppAttestEnvironment,
}

impl AppleAppAttestConfig {
    pub fn configured(&self) -> bool {
        has_text(self.team_id.as_deref()) && !self.normalized_bundle_ids().is_empty()
    }

    pub fn normalized_bundle_ids(&self) -> Vec<String> {
        self.bundle_ids
            .iter()
            .map(|bundle_id| bundle_id.trim())
            .filter(|bundle_id| !bundle_id.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub fn team_id_configured(&self) -> bool {
        has_text(self.team_id.as_deref())
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self
            .team_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            bail!("device_attestation.apple_app_attest.team_id must not be empty");
        }
        if self
            .bundle_ids
            .iter()
            .any(|bundle_id| bundle_id.trim().is_empty())
        {
            bail!("device_attestation.apple_app_attest.bundle_ids must not contain empty values");
        }
        Ok(())
    }
}

impl Default for AppleAppAttestConfig {
    fn default() -> Self {
        Self {
            mode: DeviceAttestationMode::ReportOnly,
            team_id: None,
            bundle_ids: Vec::new(),
            environment: AppAttestEnvironment::Production,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAttestationMode {
    ReportOnly,
}

impl DeviceAttestationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReportOnly => "report_only",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim() {
            "report_only" => Ok(Self::ReportOnly),
            other => bail!("unsupported device attestation mode {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAttestEnvironment {
    Development,
    Production,
}

impl AppAttestEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim() {
            "development" | "sandbox" => Ok(Self::Development),
            "production" => Ok(Self::Production),
            other => bail!("unsupported App Attest environment {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NotificationsConfig {
    pub apns_direct: ApnsDirectConfig,
    pub apns_relay: ApnsRelayConfig,
}

/// In-process APNs credentials. When these are set, the server delivers pushes
/// to Apple directly (embedding `nod-apns-relay`) with no HTTP hop or mTLS —
/// the co-located deployment. Mutually exclusive with [`ApnsRelayConfig`]; the
/// active route is decided in `push::configured_push_route`.
#[derive(Clone, Default)]
pub struct ApnsDirectConfig {
    pub bundle_id: Option<String>,
    pub team_id: Option<String>,
    pub key_id: Option<String>,
    pub private_key_path: Option<PathBuf>,
    pub environment: Option<String>,
}

impl ApnsDirectConfig {
    /// True when every credential needed to push to Apple in-process is present.
    pub fn enabled(&self) -> bool {
        has_text(self.bundle_id.as_deref())
            && has_text(self.team_id.as_deref())
            && has_text(self.key_id.as_deref())
            && path_configured(&self.private_key_path)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.any_configured() {
            return Ok(());
        }
        if !has_text(self.bundle_id.as_deref()) {
            bail!("notifications.apns_direct.bundle_id or NOD_APNS_DIRECT_BUNDLE_ID is required");
        }
        if !has_text(self.team_id.as_deref()) {
            bail!("notifications.apns_direct.team_id or NOD_APNS_DIRECT_TEAM_ID is required");
        }
        if !has_text(self.key_id.as_deref()) {
            bail!("notifications.apns_direct.key_id or NOD_APNS_DIRECT_KEY_ID is required");
        }
        if !path_configured(&self.private_key_path) {
            bail!("notifications.apns_direct.private_key_path or NOD_APNS_DIRECT_PRIVATE_KEY_PATH is required");
        }
        Ok(())
    }

    fn any_configured(&self) -> bool {
        has_text(self.bundle_id.as_deref())
            || has_text(self.team_id.as_deref())
            || has_text(self.key_id.as_deref())
            || path_configured(&self.private_key_path)
            || has_text(self.environment.as_deref())
    }
}

impl fmt::Debug for ApnsDirectConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redact Apple account identifiers so incidental Debug logging cannot expose them.
        formatter
            .debug_struct("ApnsDirectConfig")
            .field("bundle_id", &self.bundle_id)
            .field("team_id_configured", &has_text(self.team_id.as_deref()))
            .field("key_id_configured", &has_text(self.key_id.as_deref()))
            .field("private_key_path", &self.private_key_path)
            .field("environment", &self.environment)
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct ApnsRelayConfig {
    pub url: Option<String>,
    pub native_app_id: Option<String>,
    pub tls: ApnsRelayTlsConfig,
}

impl ApnsRelayConfig {
    pub fn client_enabled(&self) -> bool {
        has_text(self.url.as_deref())
            && has_text(self.native_app_id.as_deref())
            && self.tls.client_cert_configured()
            && self.tls.client_key_configured()
            && self.tls.ca_cert_configured()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.is_configured() {
            return Ok(());
        }
        if !has_text(self.url.as_deref()) {
            bail!("notifications.apns_relay.url or NOD_APNS_RELAY_URL is required");
        }
        if !has_text(self.native_app_id.as_deref()) {
            bail!("notifications.apns_relay.native_app_id or NOD_APNS_RELAY_NATIVE_APP_ID is required");
        }
        let url = url::Url::parse(self.url.as_deref().unwrap_or_default())?;
        if url.scheme() != "https" {
            bail!("notifications.apns_relay.url must use https for mTLS");
        }
        self.tls.validate()
    }

    fn is_configured(&self) -> bool {
        has_text(self.url.as_deref())
            || has_text(self.native_app_id.as_deref())
            || self.tls.any_configured()
    }
}

impl fmt::Debug for ApnsRelayConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApnsRelayConfig")
            .field("url", &self.url)
            .field("native_app_id", &self.native_app_id)
            .field("tls", &self.tls)
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct ApnsRelayTlsConfig {
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
    pub ca_cert_path: Option<PathBuf>,
}

impl ApnsRelayTlsConfig {
    pub fn client_cert_configured(&self) -> bool {
        path_configured(&self.client_cert_path)
    }

    pub fn client_key_configured(&self) -> bool {
        path_configured(&self.client_key_path)
    }

    pub fn ca_cert_configured(&self) -> bool {
        path_configured(&self.ca_cert_path)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !self.client_cert_configured() {
            bail!("NOD_APNS_RELAY_CLIENT_CERT_PATH is required");
        }
        if !self.client_key_configured() {
            bail!("NOD_APNS_RELAY_CLIENT_KEY_PATH is required");
        }
        if !self.ca_cert_configured() {
            bail!("NOD_APNS_RELAY_CA_CERT_PATH is required");
        }
        Ok(())
    }

    fn any_configured(&self) -> bool {
        self.client_cert_configured() || self.client_key_configured() || self.ca_cert_configured()
    }
}

impl fmt::Debug for ApnsRelayTlsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApnsRelayTlsConfig")
            .field("client_cert_configured", &self.client_cert_configured())
            .field("client_key_configured", &self.client_key_configured())
            .field("ca_cert_configured", &self.ca_cert_configured())
            .finish()
    }
}

#[derive(Clone, Default)]
struct ServerSecrets {
    admin_token: String,
}

impl ServerSecrets {
    fn empty() -> Self {
        Self::default()
    }

    fn admin_token(&self) -> &str {
        &self.admin_token
    }

    fn set_admin_token(&mut self, admin_token: impl Into<String>) {
        self.admin_token = admin_token.into();
    }
}

impl fmt::Debug for ServerSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerSecrets")
            .field("admin_token_configured", &has_text(Some(&self.admin_token)))
            .finish()
    }
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn path_configured(path: &Option<PathBuf>) -> bool {
    path.as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty())
}

fn default_bind() -> String {
    "127.0.0.1:8767".to_string()
}

fn default_database_url() -> String {
    "sqlite://.nod/nod.sqlite".to_string()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from(".nod")
}

fn default_retention_days() -> i64 {
    90
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apns_relay_url_requires_mtls_paths() {
        let mut config = Config::with_admin_token("admin-token");
        config.notifications.apns_relay.url = Some("https://relay.example.com".to_string());
        config.notifications.apns_relay.native_app_id = Some("com.example.NodTests".to_string());

        let err = config.validate().unwrap_err().to_string();

        assert!(err.contains("NOD_APNS_RELAY_CLIENT_CERT_PATH"));
    }

    #[test]
    fn apns_relay_url_must_use_https() {
        let mut config = Config::with_admin_token("admin-token");
        config.notifications.apns_relay.url = Some("http://relay.example.com".to_string());
        config.notifications.apns_relay.native_app_id = Some("com.example.NodTests".to_string());
        config.notifications.apns_relay.tls.client_cert_path = Some("client.crt".into());
        config.notifications.apns_relay.tls.client_key_path = Some("client.key".into());
        config.notifications.apns_relay.tls.ca_cert_path = Some("ca.crt".into());

        let err = config.validate().unwrap_err().to_string();

        assert!(err.contains("must use https"));
    }
}
