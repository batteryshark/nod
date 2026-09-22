use std::{
    collections::BTreeMap,
    env,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;

use crate::models::ServerProfile;
use crate::signing::StoredSigningKey;

const KEYRING_SERVICE: &str = "nod-client-core";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedConfig {
    #[serde(default)]
    pub servers: Vec<ServerProfile>,
    #[serde(default)]
    pub selected_server_id: Option<String>,
    #[serde(default = "default_notification_sound")]
    pub notification_sound: String,
    #[serde(default)]
    pub insecure_tokens: BTreeMap<String, String>,
    #[serde(default)]
    pub insecure_signing_keys: BTreeMap<String, StoredSigningKey>,
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            selected_server_id: None,
            notification_sound: default_notification_sound(),
            insecure_tokens: BTreeMap::new(),
            insecure_signing_keys: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    credentials: CredentialStore,
    baseline: Arc<Mutex<PersistedConfig>>,
    recovery_warning: Arc<Mutex<Option<String>>>,
}

impl Store {
    pub fn new() -> Result<Self> {
        let state_dir = if let Ok(path) = env::var("NOD_CLIENT_CORE_STATE_DIR") {
            PathBuf::from(path)
        } else {
            ProjectDirs::from("com", "Stonefish Labs", "Nod")
                .context("could not resolve user config directory")?
                .config_dir()
                .to_path_buf()
        };
        Ok(Self {
            path: state_dir.join("client-core.json"),
            credentials: CredentialStore::from_env(),
            baseline: Arc::new(Mutex::new(PersistedConfig::default())),
            recovery_warning: Arc::new(Mutex::new(None)),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_store(path: PathBuf) -> Self {
        Self {
            path,
            credentials: CredentialStore {
                use_config_file: true,
            },
            baseline: Arc::new(Mutex::new(PersistedConfig::default())),
            recovery_warning: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn load(&self) -> Result<PersistedConfig> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let config = match read_config(&store.path) {
                Ok(config) => config,
                Err(error) => {
                    let backup = store.path.with_extension("json.bak");
                    if !backup.exists() {
                        return Err(error);
                    }
                    let config = read_config(&backup)
                        .context("configuration and recovery copy are unreadable")?;
                    *store
                        .recovery_warning
                        .lock()
                        .map_err(|_| anyhow::anyhow!("store lock poisoned"))? = Some(format!(
                        "Recovered configuration from {}. Original: {error}",
                        backup.display()
                    ));
                    config
                }
            };
            *store
                .baseline
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))? = config.clone();
            Ok(config)
        })
        .await?
    }

    pub fn recovery_warning(&self) -> Option<String> {
        self.recovery_warning
            .lock()
            .ok()
            .and_then(|warning| warning.clone())
    }

    pub async fn save(&self, mut config: PersistedConfig) -> Result<()> {
        self.credentials.remove_external_credentials(&mut config);
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.save_locked(config)).await?
    }

    fn save_locked(&self, config: PersistedConfig) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("configuration has no parent directory")?;
        create_private_dir(parent)?;
        let lock_file = private_file(&self.path.with_extension("json.lock"), false)?;
        lock_file.lock().context("lock client configuration")?;
        let mut baseline = self
            .baseline
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
        let disk = read_config(&self.path).or_else(|error| {
            if self.recovery_warning().is_some() {
                read_config(&self.path.with_extension("json.bak"))
            } else {
                Err(error)
            }
        })?;
        let merged = merge_config(&baseline, &config, disk);
        let raw = serde_json::to_vec_pretty(&merged)?;
        let temporary = self
            .path
            .with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut output = private_file(&temporary, true)?;
            output.write_all(&raw)?;
            output.sync_all()?;
            if self.path.exists() && self.recovery_warning().is_none() {
                let backup = self.path.with_extension("json.bak");
                fs::copy(&self.path, &backup)?;
                restrict_permissions(&backup)?;
            }
            fs::rename(&temporary, &self.path)?;
            #[cfg(unix)]
            fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.with_context(|| format!("write {}", self.path.display()))?;
        // Keep this process's last requested state as its merge base. Changes
        // made by another shell are preserved unless this shell actually edits them.
        *baseline = config;
        Ok(())
    }

    pub async fn save_token(
        &self,
        config: &mut PersistedConfig,
        server_id: &str,
        token: &str,
    ) -> Result<()> {
        self.credentials.save_token(config, server_id, token)
    }

    pub fn load_token(&self, config: &PersistedConfig, server_id: &str) -> Option<String> {
        self.credentials.load_token(config, server_id)
    }

    pub async fn delete_token(&self, config: &mut PersistedConfig, server_id: &str) -> Result<()> {
        self.credentials.delete_token(config, server_id)
    }

    pub async fn save_signing_key(
        &self,
        config: &mut PersistedConfig,
        server_id: &str,
        signing_key: &StoredSigningKey,
    ) -> Result<()> {
        self.credentials
            .save_signing_key(config, server_id, signing_key)
    }

    pub fn load_signing_key(
        &self,
        config: &PersistedConfig,
        server_id: &str,
    ) -> Option<StoredSigningKey> {
        self.credentials.load_signing_key(config, server_id)
    }

    pub async fn delete_signing_key(
        &self,
        config: &mut PersistedConfig,
        server_id: &str,
    ) -> Result<()> {
        self.credentials.delete_signing_key(config, server_id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn read_config(path: &Path) -> Result<PersistedConfig> {
    match fs::read(path) {
        Ok(raw) => {
            serde_json::from_slice(&raw).with_context(|| format!("parse {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PersistedConfig::default())
        }
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn private_file(path: &Path, exclusive: bool) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).truncate(false);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn merge_config(
    base: &PersistedConfig,
    ours: &PersistedConfig,
    mut disk: PersistedConfig,
) -> PersistedConfig {
    for removed in base.servers.iter().filter(|server| {
        !ours
            .servers
            .iter()
            .any(|candidate| candidate.id == server.id)
    }) {
        disk.servers.retain(|server| server.id != removed.id);
    }
    for server in &ours.servers {
        if base
            .servers
            .iter()
            .find(|candidate| candidate.id == server.id)
            == Some(server)
        {
            continue;
        }
        disk.servers.retain(|candidate| candidate.id != server.id);
        disk.servers.push(server.clone());
    }
    if ours.selected_server_id != base.selected_server_id {
        disk.selected_server_id = ours.selected_server_id.clone();
    }
    if ours.notification_sound != base.notification_sound {
        disk.notification_sound = ours.notification_sound.clone();
    }
    merge_map(
        &base.insecure_tokens,
        &ours.insecure_tokens,
        &mut disk.insecure_tokens,
    );
    merge_map(
        &base.insecure_signing_keys,
        &ours.insecure_signing_keys,
        &mut disk.insecure_signing_keys,
    );
    disk
}

fn merge_map<T: Clone + PartialEq>(
    base: &BTreeMap<String, T>,
    ours: &BTreeMap<String, T>,
    disk: &mut BTreeMap<String, T>,
) {
    for key in base.keys().filter(|key| !ours.contains_key(*key)) {
        disk.remove(key);
    }
    for (key, value) in ours {
        if base.get(key) != Some(value) {
            disk.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Clone)]
struct CredentialStore {
    use_config_file: bool,
}

impl CredentialStore {
    fn from_env() -> Self {
        Self {
            use_config_file: env::var("NOD_CLIENT_CORE_INSECURE_TOKEN_STORE").is_ok(),
        }
    }

    fn remove_external_credentials(&self, config: &mut PersistedConfig) {
        if self.use_config_file {
            return;
        }
        config.insecure_tokens.clear();
        config.insecure_signing_keys.clear();
    }

    fn save_token(&self, config: &mut PersistedConfig, server_id: &str, token: &str) -> Result<()> {
        if self.use_config_file {
            config
                .insecure_tokens
                .insert(server_id.to_string(), token.to_string());
            return Ok(());
        }
        set_keyring_password(&token_account(server_id), token)
    }

    fn load_token(&self, config: &PersistedConfig, server_id: &str) -> Option<String> {
        if self.use_config_file {
            return config.insecure_tokens.get(server_id).cloned();
        }
        keyring_password(&token_account(server_id))
    }

    fn delete_token(&self, config: &mut PersistedConfig, server_id: &str) -> Result<()> {
        config.insecure_tokens.remove(server_id);
        if !self.use_config_file {
            delete_keyring_credential(&token_account(server_id));
        }
        Ok(())
    }

    fn save_signing_key(
        &self,
        config: &mut PersistedConfig,
        server_id: &str,
        signing_key: &StoredSigningKey,
    ) -> Result<()> {
        if self.use_config_file {
            config
                .insecure_signing_keys
                .insert(server_id.to_string(), signing_key.clone());
            return Ok(());
        }
        set_keyring_password(
            &signing_key_account(server_id),
            &serde_json::to_string(signing_key)?,
        )
    }

    fn load_signing_key(
        &self,
        config: &PersistedConfig,
        server_id: &str,
    ) -> Option<StoredSigningKey> {
        if self.use_config_file {
            return config.insecure_signing_keys.get(server_id).cloned();
        }
        keyring_password(&signing_key_account(server_id))
            .and_then(|raw| serde_json::from_str(&raw).ok())
    }

    fn delete_signing_key(&self, config: &mut PersistedConfig, server_id: &str) -> Result<()> {
        config.insecure_signing_keys.remove(server_id);
        if !self.use_config_file {
            delete_keyring_credential(&signing_key_account(server_id));
        }
        Ok(())
    }
}

fn set_keyring_password(account: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
    entry.set_password(password)?;
    Ok(())
}

fn keyring_password(account: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    entry.get_password().ok()
}

fn delete_keyring_credential(account: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account) {
        let _ = entry.delete_credential();
    }
}

fn token_account(server_id: &str) -> String {
    format!("serverToken.{server_id}")
}

fn signing_key_account(server_id: &str) -> String {
    format!("decisionSigningKey.{server_id}")
}

fn default_notification_sound() -> String {
    "default".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str) -> ServerProfile {
        ServerProfile {
            id: id.into(),
            credential_id: None,
            name: id.into(),
            base_url_string: format!("https://{id}.example"),
            device_name: "test".into(),
            device_id: None,
            user_id: None,
            user_name: None,
        }
    }

    #[tokio::test]
    async fn concurrent_shells_preserve_unrelated_edits_and_removals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client-core.json");
        let first = Store::test_store(path.clone());
        let second = Store::test_store(path.clone());
        let mut initial = first.load().await.unwrap();
        initial.servers.push(profile("existing"));
        first.save(initial.clone()).await.unwrap();
        let mut other = second.load().await.unwrap();
        initial.servers.push(profile("new"));
        first.save(initial.clone()).await.unwrap();
        other.servers.clear();
        other.notification_sound = "silent".into();
        second.save(other).await.unwrap();
        initial.notification_sound = "default".into();
        first.save(initial).await.unwrap();
        let final_state = read_config(&path).unwrap();
        assert_eq!(
            final_state
                .servers
                .iter()
                .map(|server| server.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new"]
        );
        assert_eq!(final_state.notification_sound, "silent");
    }

    #[tokio::test]
    async fn truncated_config_recovers_last_valid_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client-core.json");
        let store = Store::test_store(path.clone());
        let mut config = store.load().await.unwrap();
        config.servers.push(profile("saved"));
        store.save(config.clone()).await.unwrap();
        config.notification_sound = "silent".into();
        store.save(config).await.unwrap();
        fs::write(&path, b"{broken").unwrap();
        let reopened = Store::test_store(path);
        let recovered = reopened.load().await.unwrap();
        assert_eq!(recovered.servers[0].id, "saved");
        assert!(reopened.recovery_warning().is_some());
        reopened.save(recovered).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn insecure_storage_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client-core.json");
        Store::test_store(path.clone())
            .save(PersistedConfig::default())
            .await
            .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
