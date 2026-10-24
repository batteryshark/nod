use serde::Deserialize;
use std::{fmt, path::PathBuf};

#[derive(Clone, Default, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntegrationsConfig {
    pub nats: NatsConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NatsConfig {
    pub url: Option<String>,
    pub credentials_file: Option<PathBuf>,
    pub token_file: Option<PathBuf>,
    pub user: Option<String>,
    pub password_file: Option<PathBuf>,
    pub inbox_prefix: Option<String>,
    pub expected_stream: String,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: None,
            credentials_file: None,
            token_file: None,
            user: None,
            password_file: None,
            inbox_prefix: None,
            expected_stream: "TOWER".into(),
        }
    }
}

impl NatsConfig {
    pub fn configured(&self) -> bool {
        self.url.is_some()
            || self.credentials_file.is_some()
            || self.token_file.is_some()
            || self.user.is_some()
            || self.password_file.is_some()
            || self.inbox_prefix.is_some()
    }
    pub fn valid(&self) -> bool {
        self.url
            .as_deref()
            .and_then(|v| url::Url::parse(v).ok())
            .is_some_and(|u| {
                matches!(u.scheme(), "nats" | "tls")
                    && u.host_str().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none()
            })
            && match (
                &self.credentials_file,
                &self.token_file,
                &self.user,
                &self.password_file,
            ) {
                (Some(path), None, None, None) | (None, Some(path), None, None) => {
                    !path.as_os_str().is_empty()
                }
                (None, None, Some(user), Some(path)) => {
                    !path.as_os_str().is_empty()
                        && !user.is_empty()
                        && !user.chars().any(|c| c.is_whitespace() || c.is_control())
                }
                _ => false,
            }
            && self.inbox_prefix.as_ref().is_none_or(|prefix| {
                !prefix.is_empty()
                    && prefix
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            })
            && !self.expected_stream.is_empty()
            && self
                .expected_stream
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    }
}

impl fmt::Debug for NatsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NatsConfig")
            .field("configured", &self.configured())
            .field("valid", &self.valid())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_mode_and_safe_inbox_are_exclusive_and_redacted() {
        let mut config = NatsConfig {
            url: Some("nats://127.0.0.1:12345".into()),
            user: Some("nod".into()),
            ..Default::default()
        };
        assert!(!config.valid());
        config.password_file = Some("/dummy/password".into());
        config.inbox_prefix = Some("_INBOX_nod".into());
        assert!(config.valid());
        assert!(!format!("{config:?}").contains("password"));
        config.token_file = Some("dummy".into());
        assert!(!config.valid());
        config.token_file = None;
        config.inbox_prefix = Some("_INBOX.>".into());
        assert!(!config.valid());
    }
    #[test]
    fn auth_file_alternatives_are_exclusive_and_debug_is_redacted() {
        let mut config = NatsConfig {
            url: Some("nats://localhost:4222".into()),
            ..Default::default()
        };
        assert!(!config.valid());
        config.token_file = Some("/private/dummy-token-file".into());
        assert!(config.valid());
        let debug = format!("{config:?}");
        assert!(!debug.contains("private"));
        assert!(!debug.contains("localhost"));
        config.credentials_file = Some("/private/dummy-creds-file".into());
        assert!(!config.valid());
        config.token_file = None;
        assert!(config.valid());
        config.credentials_file = None;
        config.token_file = Some(PathBuf::new());
        assert!(!config.valid());
        config.url = None;
        assert!(config.configured());
        assert!(!config.valid());
    }
    #[test]
    fn opt_in_and_redaction() {
        let mut config = NatsConfig::default();
        assert!(!config.configured());
        config.url = Some("nats://private-user:private-secret@localhost:4222".into());
        config.credentials_file = Some("/private/secret.creds".into());
        assert!(!config.valid());
        let debug = format!("{config:?}");
        assert!(!debug.contains("private"));
        config.url = Some("nats://localhost:4222".into());
        assert!(config.valid());
        config.expected_stream = "*".into();
        assert!(!config.valid());
    }
}
