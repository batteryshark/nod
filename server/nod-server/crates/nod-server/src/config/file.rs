use std::{fs, path::Path};

use anyhow::Context;
use serde::Deserialize;

use super::{
    ApnsDirectConfig, ApnsRelayConfig, ApnsRelayTlsConfig, AppAttestEnvironment,
    AppleAppAttestConfig, Config, DeviceAttestationConfig, DeviceAttestationMode,
    NotificationsConfig,
};

pub(super) fn load_server_config(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    server_from_toml(&raw)
        .with_context(|| format!("failed to parse config file {}", path.display()))
}

fn server_from_toml(raw: &str) -> anyhow::Result<Config> {
    let file_config = toml::from_str::<ServerConfigFile>(raw)?;
    file_config.into_config()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfigFile {
    bind: Option<String>,
    database_url: Option<String>,
    data_dir: Option<std::path::PathBuf>,
    retention_days: Option<i64>,
    callback_allowed_origins: Option<Vec<String>>,
    #[serde(default)]
    notifications: NotificationsConfigFile,
    #[serde(default)]
    device_attestation: DeviceAttestationConfigFile,
}

impl ServerConfigFile {
    fn into_config(self) -> anyhow::Result<Config> {
        let mut config = Config::without_secrets();
        if let Some(bind) = self.bind {
            config.bind = bind;
        }
        if let Some(database_url) = self.database_url {
            config.database_url = database_url;
        }
        if let Some(data_dir) = self.data_dir {
            config.data_dir = data_dir;
        }
        if let Some(retention_days) = self.retention_days {
            config.retention_days = retention_days;
        }
        config.notifications = self.notifications.into_config();
        config.callback_allowed_origins = self.callback_allowed_origins;
        config.device_attestation = self.device_attestation.into_config()?;
        Ok(config)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceAttestationConfigFile {
    #[serde(default)]
    apple_app_attest: AppleAppAttestConfigFile,
}

impl DeviceAttestationConfigFile {
    fn into_config(self) -> anyhow::Result<DeviceAttestationConfig> {
        Ok(DeviceAttestationConfig {
            apple_app_attest: self.apple_app_attest.into_config()?,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppleAppAttestConfigFile {
    mode: Option<String>,
    team_id: Option<String>,
    #[serde(default)]
    bundle_ids: Vec<String>,
    environment: Option<String>,
}

impl AppleAppAttestConfigFile {
    fn into_config(self) -> anyhow::Result<AppleAppAttestConfig> {
        let mut config = AppleAppAttestConfig::default();
        if let Some(mode) = self.mode {
            config.mode = DeviceAttestationMode::parse(&mode)?;
        }
        if let Some(team_id) = self.team_id {
            config.team_id = Some(team_id);
        }
        config.bundle_ids = self.bundle_ids;
        if let Some(environment) = self.environment {
            config.environment = AppAttestEnvironment::parse(&environment)?;
        }
        Ok(config)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationsConfigFile {
    #[serde(default)]
    apns_direct: ApnsDirectConfigFile,
    #[serde(default)]
    apns_relay: ApnsRelayConfigFile,
}

impl NotificationsConfigFile {
    fn into_config(self) -> NotificationsConfig {
        NotificationsConfig {
            apns_direct: self.apns_direct.into_config(),
            apns_relay: self.apns_relay.into_config(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApnsDirectConfigFile {
    bundle_id: Option<String>,
    team_id: Option<String>,
    key_id: Option<String>,
    private_key_path: Option<std::path::PathBuf>,
    environment: Option<String>,
}

impl ApnsDirectConfigFile {
    fn into_config(self) -> ApnsDirectConfig {
        ApnsDirectConfig {
            bundle_id: self.bundle_id,
            team_id: self.team_id,
            key_id: self.key_id,
            private_key_path: self.private_key_path,
            environment: self.environment,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApnsRelayConfigFile {
    url: Option<String>,
    native_app_id: Option<String>,
    client_cert_path: Option<std::path::PathBuf>,
    client_key_path: Option<std::path::PathBuf>,
    ca_cert_path: Option<std::path::PathBuf>,
}

impl ApnsRelayConfigFile {
    fn into_config(self) -> ApnsRelayConfig {
        ApnsRelayConfig {
            url: self.url,
            native_app_id: self.native_app_id,
            tls: ApnsRelayTlsConfig {
                client_cert_path: self.client_cert_path,
                client_key_path: self.client_key_path,
                ca_cert_path: self.ca_cert_path,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_file_accepts_only_non_secret_settings() {
        let config = server_from_toml(
            r#"
            bind = "0.0.0.0:8767"
            database_url = "sqlite:///data/nod.sqlite"
            data_dir = "/data"
            retention_days = 30

            [notifications.apns_relay]
            url = "https://relay.example.com"
            native_app_id = "com.example.Nod"
            client_cert_path = "/secrets/relay-client.crt"
            client_key_path = "/secrets/relay-client.key"
            ca_cert_path = "/secrets/relay-ca.crt"

            [device_attestation.apple_app_attest]
            mode = "report_only"
            team_id = "TEAMID"
            bundle_ids = ["com.example.Nod", "com.example.NodMac"]
            environment = "production"
            "#,
        )
        .unwrap();

        assert_eq!(config.bind, "0.0.0.0:8767");
        assert_eq!(config.database_url, "sqlite:///data/nod.sqlite");
        assert_eq!(config.data_dir, std::path::PathBuf::from("/data"));
        assert_eq!(config.retention_days, 30);
        assert_eq!(
            config.notifications.apns_relay.url.as_deref(),
            Some("https://relay.example.com")
        );
        assert_eq!(
            config.notifications.apns_relay.native_app_id.as_deref(),
            Some("com.example.Nod")
        );
        assert_eq!(
            config
                .notifications
                .apns_relay
                .tls
                .client_cert_path
                .as_deref(),
            Some(std::path::Path::new("/secrets/relay-client.crt"))
        );
        assert_eq!(
            config.device_attestation.apple_app_attest.mode.as_str(),
            "report_only"
        );
        assert_eq!(
            config
                .device_attestation
                .apple_app_attest
                .team_id
                .as_deref(),
            Some("TEAMID")
        );
        assert_eq!(
            config
                .device_attestation
                .apple_app_attest
                .normalized_bundle_ids(),
            vec!["com.example.Nod", "com.example.NodMac"]
        );
        assert_eq!(
            config
                .device_attestation
                .apple_app_attest
                .environment
                .as_str(),
            "production"
        );
        assert_eq!(config.admin_token(), "");
        assert!(config.notifications.apns_relay.client_enabled());
    }

    #[test]
    fn server_config_file_rejects_secrets() {
        let err = server_from_toml(
            r#"
            admin_token = "do-not-put-this-here"
            "#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("unknown field"));
    }
}
