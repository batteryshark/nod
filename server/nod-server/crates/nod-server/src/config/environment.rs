use std::{env, fs, path::PathBuf};

use anyhow::{bail, Context};

use super::{
    ApnsDirectConfig, ApnsRelayConfig, AppAttestEnvironment, AppleAppAttestConfig, Config,
    DeviceAttestationMode,
};

pub(super) fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn apply_server_env(config: &mut Config) -> anyhow::Result<()> {
    if let Some(value) = raw_env("NOD_BIND") {
        config.bind = value;
    }
    if let Some(value) = raw_env("NOD_DATABASE_URL") {
        config.database_url = value;
    }
    if let Some(value) = raw_env("NOD_DATA_DIR") {
        config.data_dir = PathBuf::from(value);
    }
    if let Some(value) = raw_env("NOD_RETENTION_DAYS") {
        config.retention_days = value
            .parse()
            .with_context(|| format!("NOD_RETENTION_DAYS must be an integer, got {value:?}"))?;
    }
    if let Some(value) = injected_value("NOD_ADMIN_TOKEN")? {
        config.secrets.set_admin_token(value);
    }
    if let Some(value) = raw_env("NOD_CALLBACK_ALLOWED_ORIGINS") {
        config.callback_allowed_origins = Some(split_secret_list(&value));
    }
    apply_apns_direct_env(&mut config.notifications.apns_direct)?;
    apply_apns_relay_env(&mut config.notifications.apns_relay)?;
    apply_apple_app_attest_env(&mut config.device_attestation.apple_app_attest)?;
    Ok(())
}

fn apply_apns_direct_env(config: &mut ApnsDirectConfig) -> anyhow::Result<()> {
    if let Some(value) = raw_env("NOD_APNS_DIRECT_BUNDLE_ID") {
        config.bundle_id = Some(value);
    }
    if let Some(value) = raw_env("NOD_APNS_DIRECT_TEAM_ID") {
        config.team_id = Some(value);
    }
    if let Some(value) = raw_env("NOD_APNS_DIRECT_KEY_ID") {
        config.key_id = Some(value);
    }
    if let Some(value) = injected_value("NOD_APNS_DIRECT_PRIVATE_KEY_PATH")? {
        config.private_key_path = Some(PathBuf::from(value));
    }
    if let Some(value) = raw_env("NOD_APNS_DIRECT_ENVIRONMENT") {
        config.environment = Some(value);
    }
    Ok(())
}

fn apply_apns_relay_env(config: &mut ApnsRelayConfig) -> anyhow::Result<()> {
    if let Some(value) = raw_env("NOD_APNS_RELAY_URL") {
        config.url = Some(value);
    }
    if let Some(value) = raw_env("NOD_APNS_RELAY_NATIVE_APP_ID") {
        config.native_app_id = Some(value);
    }
    if let Some(value) = injected_value("NOD_APNS_RELAY_CLIENT_CERT_PATH")? {
        config.tls.client_cert_path = Some(PathBuf::from(value));
    }
    if let Some(value) = injected_value("NOD_APNS_RELAY_CLIENT_KEY_PATH")? {
        config.tls.client_key_path = Some(PathBuf::from(value));
    }
    if let Some(value) = injected_value("NOD_APNS_RELAY_CA_CERT_PATH")? {
        config.tls.ca_cert_path = Some(PathBuf::from(value));
    }
    Ok(())
}

fn apply_apple_app_attest_env(config: &mut AppleAppAttestConfig) -> anyhow::Result<()> {
    if let Some(value) = raw_env("NOD_APPLE_APP_ATTEST_MODE") {
        config.mode = DeviceAttestationMode::parse(&value)?;
    }
    if let Some(value) = raw_env("NOD_APPLE_APP_ATTEST_TEAM_ID") {
        config.team_id = Some(value);
    }
    if let Some(value) = raw_env("NOD_APPLE_APP_ATTEST_BUNDLE_IDS") {
        config.bundle_ids = split_secret_list(&value);
    }
    if let Some(value) = raw_env("NOD_APPLE_APP_ATTEST_ENVIRONMENT") {
        config.environment = AppAttestEnvironment::parse(&value)?;
    }
    Ok(())
}

fn raw_env(name: &str) -> Option<String> {
    env::var(name).ok().map(|value| value.trim().to_string())
}

fn injected_value(name: &str) -> anyhow::Result<Option<String>> {
    let direct = env::var(name).ok();
    let file_var = format!("{name}_FILE");
    let file_path = optional_env(&file_var);

    match (direct, file_path) {
        (Some(_), Some(_)) => bail!("set either {name} or {file_var}, not both"),
        (Some(value), None) => non_empty_value(name, value),
        (None, Some(path)) => read_injected_value(name, &path),
        (None, None) => Ok(None),
    }
}

fn read_injected_value(name: &str, path: &str) -> anyhow::Result<Option<String>> {
    // The *_FILE variants read file contents as the injected value,
    // matching common container secret mounts.
    let value = fs::read_to_string(path)
        .with_context(|| format!("failed to read {name}_FILE secret from {path}"))?;
    non_empty_value(name, value)
}

fn non_empty_value(name: &str, value: String) -> anyhow::Result<Option<String>> {
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(Some(value))
}

fn split_secret_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
