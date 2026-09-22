use anyhow::{anyhow, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{DecisionSignature, DeviceSigningKey, OptionKind, Request, RequestOption};

/// The decision-signing algorithm id, mirrored from nod-proto.
pub const DECISION_SIGNING_ALGORITHM: &str = nod_proto::DECISION_SIGNING_ALGORITHM;

const IMPLICIT_DISMISS_OPTION_ID: &str = "dismiss";

/// A locally stored P-256 signing key (key id + base64url private key). The
/// cryptography lives in `nod-proto`; this just wraps the stored key material
/// and orchestrates option resolution and nonce/timestamp generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSigningKey {
    pub key_id: String,
    pub private_key: String,
}

impl StoredSigningKey {
    pub fn generate() -> Self {
        let keypair = nod_proto::generate_signing_key();
        Self {
            key_id: uuid::Uuid::new_v4().to_string(),
            private_key: keypair.private_key,
        }
    }
}

/// A device's decision-signing primitive. The software path
/// (`StoredSigningKey`, used by the TUI + desktop) and the Apple Secure Enclave
/// path (a foreign signer reached over UniFFI) both implement this; the runtime
/// never cares which it holds. The trait is deliberately *just* the primitive —
/// identity (`key_id`/`algorithm`/`public_key`) and "sign these bytes" — while
/// `build_decision_signature` owns the security-critical orchestration (digest
/// recompute, option resolution, nonce/timestamp, canonical payload).
pub trait DeviceSigner: Send + Sync {
    /// The server-registered key id echoed back in every decision.
    fn key_id(&self) -> String;
    /// The signature algorithm id. Both paths are P-256 ECDSA / SHA-256.
    fn algorithm(&self) -> String {
        DECISION_SIGNING_ALGORITHM.to_string()
    }
    /// base64url uncompressed (x9.63) P-256 public key, sent at enrollment.
    fn public_key(&self) -> Result<String>;
    /// Sign the canonical decision payload bytes; returns a base64url DER ECDSA
    /// signature. For the Secure Enclave this is the only step that crosses into
    /// hardware — the bytes are built in Rust and never leave as a private key.
    fn sign(&self, payload: &[u8]) -> Result<String>;

    /// The public key bundle registered with the server at enrollment, built
    /// uniformly from any signer's identity.
    fn device_signing_key(&self) -> Result<DeviceSigningKey> {
        Ok(DeviceSigningKey {
            key_id: self.key_id(),
            algorithm: self.algorithm(),
            public_key: self.public_key()?,
        })
    }
}

impl DeviceSigner for StoredSigningKey {
    fn key_id(&self) -> String {
        self.key_id.clone()
    }

    fn public_key(&self) -> Result<String> {
        Ok(nod_proto::public_key_for(&self.private_key)?)
    }

    fn sign(&self, payload: &[u8]) -> Result<String> {
        Ok(nod_proto::sign_payload(&self.private_key, payload)?)
    }
}

/// Build a verified `DecisionSignature` for `request` using any `DeviceSigner`.
///
/// This is the single security-critical path shared by every platform: it
/// refuses to sign unless it can reproduce the request digest from the content
/// it received (defense in depth against a server lying about what is being
/// approved), resolves the option, stamps a fresh nonce + timestamp, builds the
/// canonical `nod-proto` payload, and asks the signer to sign those exact bytes.
pub fn build_decision_signature(
    signer: &dyn DeviceSigner,
    request: DecisionSigningRequest<'_>,
) -> Result<DecisionSignature> {
    // Defense in depth: only sign a request whose digest we can reproduce
    // from the content we received (and rendered to the user), rather than
    // trusting the digest the server asserted.
    let (request_digest, recomputed) =
        if let Some(context) = &request.request.signing {
            if context.version != nod_proto::PRIVATE_REQUEST_DIGEST_VERSION {
                return Err(anyhow!(
                    "unsupported request signing version: {}",
                    context.version
                ));
            }
            (
                context.request_digest.as_str(),
                nod_proto::request_digest_v2(request.request, &context.recipients_commitment)?,
            )
        } else {
            (
                request.request.request_digest.as_deref().ok_or_else(|| {
                    anyhow!("request digest is missing for {}", request.request.id)
                })?,
                nod_proto::request_digest(request.request)?,
            )
        };
    if recomputed != request_digest {
        return Err(anyhow!(
            "request digest does not match request content for {}",
            request.request.id
        ));
    }
    let option = option_for(request.request, request.option_id)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let signed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let text = request.text.and_then(trimmed_text);
    let key_id = signer.key_id();
    let payload = nod_proto::decision_signing_payload(nod_proto::DecisionSigningInput {
        request_id: &request.request.id,
        request_digest,
        option_id: option.id,
        option_kind: option.kind,
        user_id: request.user_id,
        device_id: request.device_id,
        key_id: &key_id,
        nonce: &nonce,
        signed_at: &signed_at,
        text,
    });
    let signature = signer.sign(payload.as_bytes())?;
    Ok(DecisionSignature {
        key_id,
        algorithm: signer.algorithm(),
        nonce,
        signed_at,
        request_digest: request_digest.to_string(),
        signature,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct DecisionSigningRequest<'a> {
    pub request: &'a Request,
    pub option_id: &'a str,
    pub text: Option<&'a str>,
    pub user_id: &'a str,
    pub device_id: &'a str,
}

/// Identity of a host-provisioned hardware key (Apple Secure Enclave). The
/// private key never crosses this boundary — only the id the server registers
/// and the public key it verifies against.
#[derive(Debug, Clone)]
pub struct ForeignSignerKey {
    pub key_id: String,
    /// base64url uncompressed (x9.63) P-256 public key.
    pub public_key: String,
}

/// A host-owned signing backend (the Apple Secure Enclave). The runtime calls
/// out to it instead of generating/persisting a software key, so the Apple apps
/// keep non-exportable hardware keys while still using all of nod-client-core.
/// Keyed by server profile id because each enrolled server has its own device
/// key. Implemented in `nod-client-ffi` by a UniFFI foreign callback to Swift.
pub trait ForeignSigner: Send + Sync {
    /// Create (or fetch) the hardware key for a freshly enrolled profile and
    /// return its public identity to register with the server.
    fn provision(&self, profile_id: &str) -> Result<ForeignSignerKey>;
    /// The existing hardware key for a profile, or `None` if there is none
    /// (in which case decisions for that server cannot be signed).
    fn signing_key(&self, profile_id: &str) -> Result<Option<ForeignSignerKey>>;
    /// Sign the canonical decision payload bytes with the profile's hardware key
    /// (base64url DER ECDSA). The bytes are built in Rust by
    /// `build_decision_signature`.
    fn sign(&self, profile_id: &str, payload: &[u8]) -> Result<String>;
    /// Drop the hardware key when a server is forgotten.
    fn remove(&self, profile_id: &str) -> Result<()>;
}

/// Adapts a `ForeignSigner` + a specific profile's resolved key into a
/// `DeviceSigner`, so `build_decision_signature` treats hardware and software
/// keys identically.
pub struct ForeignDeviceSigner {
    pub backend: std::sync::Arc<dyn ForeignSigner>,
    pub profile_id: String,
    pub key: ForeignSignerKey,
}

impl DeviceSigner for ForeignDeviceSigner {
    fn key_id(&self) -> String {
        self.key.key_id.clone()
    }

    fn public_key(&self) -> Result<String> {
        Ok(self.key.public_key.clone())
    }

    fn sign(&self, payload: &[u8]) -> Result<String> {
        self.backend.sign(&self.profile_id, payload)
    }
}

#[derive(Debug, Clone, Copy)]
struct OptionRef<'a> {
    id: &'a str,
    kind: &'a OptionKind,
}

impl<'a> From<&'a RequestOption> for OptionRef<'a> {
    fn from(option: &'a RequestOption) -> Self {
        Self {
            id: &option.id,
            kind: &option.kind,
        }
    }
}

fn option_for<'a>(request: &'a Request, option_id: &'a str) -> Result<OptionRef<'a>> {
    if let Some(option) = request.options.iter().find(|option| option.id == option_id) {
        return Ok(option.into());
    }

    // The UI can synthesize a dismiss button for optionless requests. The server
    // still verifies that submission against the same canonical payload shape.
    if option_id == IMPLICIT_DISMISS_OPTION_ID && request.options.is_empty() {
        return Ok(OptionRef {
            id: IMPLICIT_DISMISS_OPTION_ID,
            kind: &OptionKind::Dismiss,
        });
    }
    Err(anyhow!(
        "option {option_id} is not available for {}",
        request.id
    ))
}

fn trimmed_text(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::models::{DecisionResolution, RequestStatus};

    fn request() -> Request {
        let mut request = Request {
            id: "request-1".to_string(),
            request_id: "request-1".to_string(),
            channel_id: "default".to_string(),
            recipients: Vec::new(),
            decision_resolution: DecisionResolution::Shared,
            title: "Deploy".to_string(),
            summary: String::new(),
            body_markdown: String::new(),
            fields: Vec::new(),
            links: Vec::new(),
            image_url: None,
            notification: Default::default(),
            dedupe_key: None,
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: None,
            options: vec![RequestOption {
                id: "approve".to_string(),
                label: "Approve".to_string(),
                kind: OptionKind::Approve,
                style: "default".to_string(),
                requires_text: false,
                text_placeholder: None,
                destructive: false,
                foreground: false,
            }],
            request_digest: None,
            signing: None,
        };
        request.request_digest = Some(nod_proto::request_digest(&request).unwrap());
        request
    }

    #[test]
    fn optionless_requests_can_still_dismiss() {
        let mut request = request();
        request.options.clear();

        let option = option_for(&request, IMPLICIT_DISMISS_OPTION_ID)
            .expect("implicit dismiss should be available");

        assert_eq!(option.id, IMPLICIT_DISMISS_OPTION_ID);
        assert_eq!(*option.kind, OptionKind::Dismiss);
    }

    #[test]
    fn blank_decision_text_is_signed_as_empty() {
        assert_eq!(trimmed_text("  \n\t  "), None);
    }

    #[test]
    fn signing_fails_when_request_digest_is_missing() {
        let mut request = request();
        request.request_digest = None;
        let key = StoredSigningKey::generate();
        let error = build_decision_signature(
            &key,
            DecisionSigningRequest {
                request: &request,
                option_id: "approve",
                text: None,
                user_id: "user-1",
                device_id: "device-1",
            },
        )
        .expect_err("missing digest should fail signing");

        assert!(error.to_string().contains("request digest is missing"));
    }

    #[test]
    fn signing_fails_when_option_is_not_available() {
        let request = request();
        let key = StoredSigningKey::generate();
        let error = build_decision_signature(
            &key,
            DecisionSigningRequest {
                request: &request,
                option_id: "reject",
                text: None,
                user_id: "user-1",
                device_id: "device-1",
            },
        )
        .expect_err("unknown option should fail signing");

        assert!(error.to_string().contains("option reject is not available"));
    }

    #[test]
    fn signing_fails_when_request_content_does_not_match_digest() {
        let mut request = request();
        request.title = "Tampered after the digest was computed".to_string();
        let key = StoredSigningKey::generate();
        let error = build_decision_signature(
            &key,
            DecisionSigningRequest {
                request: &request,
                option_id: "approve",
                text: None,
                user_id: "user-1",
                device_id: "device-1",
            },
        )
        .expect_err("a stale digest must fail signing");

        assert!(error.to_string().contains("does not match request content"));
    }

    #[test]
    fn private_signing_rejects_tampered_content_and_unknown_versions() {
        let mut request = request();
        let commitment =
            nod_proto::recipients_commitment(&request.recipients, "test-salt").unwrap();
        request.signing = Some(nod_proto::RequestSigningContext {
            version: nod_proto::PRIVATE_REQUEST_DIGEST_VERSION.to_string(),
            request_digest: nod_proto::request_digest_v2(&request, &commitment).unwrap(),
            recipients_commitment: commitment,
        });
        let key = StoredSigningKey::generate();
        request.title.push_str(" forged");
        let sign = |request: &Request| {
            build_decision_signature(
                &key,
                DecisionSigningRequest {
                    request,
                    option_id: "approve",
                    text: None,
                    user_id: "owner",
                    device_id: "device",
                },
            )
        };
        assert!(sign(&request)
            .unwrap_err()
            .to_string()
            .contains("does not match request content"));
        request.signing.as_mut().unwrap().version = "nod-request-v999".to_string();
        assert!(sign(&request)
            .unwrap_err()
            .to_string()
            .contains("unsupported request signing version"));
    }

    #[test]
    fn generated_public_key_uses_uncompressed_x963_bytes() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        let key = StoredSigningKey::generate();
        let public_key = key
            .device_signing_key()
            .expect("generated key should expose a public key");
        let bytes = URL_SAFE_NO_PAD
            .decode(public_key.public_key)
            .expect("public key should be base64url encoded");

        assert_eq!(bytes.len(), 65);
        assert_eq!(bytes[0], 4);
    }

    /// A `ForeignSigner` (the Secure Enclave seam) must drive the exact same
    /// canonical path: `build_decision_signature` builds the bytes, hands them to
    /// the backend, and the resulting signature verifies against the backend's
    /// public key over those exact bytes. Emulates the SE with a software key so
    /// the test can assert real signature verification.
    #[test]
    fn foreign_signer_path_produces_verifiable_signature() {
        use std::sync::{Arc, Mutex};

        struct FakeSecureEnclave {
            key: StoredSigningKey,
            signed_payload: Arc<Mutex<Option<Vec<u8>>>>,
        }

        impl ForeignSigner for FakeSecureEnclave {
            fn provision(&self, _profile_id: &str) -> Result<ForeignSignerKey> {
                self.signing_key(_profile_id).map(|k| k.unwrap())
            }
            fn signing_key(&self, _profile_id: &str) -> Result<Option<ForeignSignerKey>> {
                Ok(Some(ForeignSignerKey {
                    key_id: self.key.key_id(),
                    public_key: self.key.public_key()?,
                }))
            }
            fn sign(&self, _profile_id: &str, payload: &[u8]) -> Result<String> {
                // Capture the exact bytes the runtime asked the hardware to sign.
                *self.signed_payload.lock().unwrap() = Some(payload.to_vec());
                self.key.sign(payload)
            }
            fn remove(&self, _profile_id: &str) -> Result<()> {
                Ok(())
            }
        }

        let software = StoredSigningKey::generate();
        let public_key = software.public_key().unwrap();
        let signed_payload = Arc::new(Mutex::new(None));
        let backend: Arc<dyn ForeignSigner> = Arc::new(FakeSecureEnclave {
            key: software,
            signed_payload: signed_payload.clone(),
        });

        let key = backend.signing_key("p1").unwrap().unwrap();
        let signer = ForeignDeviceSigner {
            backend: backend.clone(),
            profile_id: "p1".to_string(),
            key,
        };

        let request = request();
        let signature = build_decision_signature(
            &signer,
            DecisionSigningRequest {
                request: &request,
                option_id: "approve",
                text: None,
                user_id: "user-1",
                device_id: "device-1",
            },
        )
        .expect("foreign signer should produce a signature");

        assert_eq!(signature.key_id, signer.key_id());
        assert_eq!(signature.algorithm, DECISION_SIGNING_ALGORITHM);
        let payload = signed_payload
            .lock()
            .unwrap()
            .clone()
            .expect("backend was asked to sign");
        nod_proto::verify_payload(&public_key, &payload, &signature.signature)
            .expect("the foreign signature must verify against the backend public key");
    }
}
