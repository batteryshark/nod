use std::{
    fs,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;

use crate::config::ApnsCredentials;

// Apple rejects a provider token that is refreshed more than once every 20
// minutes (`TooManyProviderTokenUpdates`) and one that is older than 60 minutes
// (`ExpiredProviderToken`). Reuse a single token well inside that window and
// only mint a new one once it ages past this threshold.
const TOKEN_REFRESH_AFTER_SECS: u64 = 50 * 60;

pub(crate) struct ApnsTokenSigner {
    team_id: String,
    key_id: String,
    encoding_key: EncodingKey,
    cached: Mutex<Option<CachedToken>>,
}

struct CachedToken {
    jwt: String,
    issued_at: u64,
}

impl ApnsTokenSigner {
    pub(crate) fn from_credentials(credentials: ApnsCredentials) -> anyhow::Result<Self> {
        let key = fs::read(credentials.private_key_path)?;
        Ok(Self {
            team_id: credentials.team_id,
            key_id: credentials.key_id,
            encoding_key: EncodingKey::from_ec_pem(&key)?,
            cached: Mutex::new(None),
        })
    }

    /// Return a provider token, reusing the cached one until it ages past
    /// [`TOKEN_REFRESH_AFTER_SECS`]. Minting a fresh token per push is what trips
    /// Apple's `429 TooManyProviderTokenUpdates`, so the token is signed only
    /// when there is no cached one or the cached one is stale.
    pub(crate) fn jwt(&self) -> anyhow::Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(token) = cached.as_ref() {
            if now.saturating_sub(token.issued_at) < TOKEN_REFRESH_AFTER_SECS {
                return Ok(token.jwt.clone());
            }
        }

        let jwt = self.sign(now)?;
        *cached = Some(CachedToken {
            jwt: jwt.clone(),
            issued_at: now,
        });
        Ok(jwt)
    }

    fn sign(&self, issued_at: u64) -> anyhow::Result<String> {
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            iat: u64,
        }

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());

        Ok(jsonwebtoken::encode(
            &header,
            &Claims {
                iss: &self.team_id,
                iat: issued_at,
            },
            &self.encoding_key,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ApnsCredentials;

    use super::*;

    fn signer() -> ApnsTokenSigner {
        ApnsTokenSigner::from_credentials(ApnsCredentials {
            team_id: "TEAMID".to_string(),
            key_id: "KEYID".to_string(),
            private_key_path: "tests/fixtures/mtls/apns-auth-key.p8".into(),
        })
        .unwrap()
    }

    #[test]
    fn reuses_cached_token_across_calls() {
        let signer = signer();
        let first = signer.jwt().unwrap();
        let second = signer.jwt().unwrap();
        // A fresh ES256 signature would differ each call; identical output proves
        // the cached token (not a new one) was returned.
        assert_eq!(first, second);
    }

    #[test]
    fn refreshes_token_once_stale() {
        let signer = signer();
        let original = signer.jwt().unwrap();

        // Backdate the cached token past the refresh window so the next call mints
        // a new one.
        {
            let mut cached = signer.cached.lock().unwrap();
            let entry = cached.as_mut().unwrap();
            entry.issued_at = entry.issued_at.saturating_sub(TOKEN_REFRESH_AFTER_SECS + 1);
        }

        let refreshed = signer.jwt().unwrap();
        assert_ne!(original, refreshed);
    }
}
