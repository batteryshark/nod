//! The ONE UniFFI surface for the Nod client — how the Apple apps reach all
//! shared Rust.
//!
//! The Apple apps run on `nod-client-core` (the same client the TUI + Tauri
//! desktop use); NodKit's formerly duplicated Swift client was deleted. Swift
//! keeps only the genuinely-native adapters: Secure Enclave signing, App Attest,
//! UserNotifications + APNs token, and SwiftUI.
//! See `.project/decisions.md` ("Apple apps move onto nod-client-core").
//!
//! Two surfaces, split by file:
//!   - this file — the stateless decision-signing contract from `nod-proto`
//!     (`request_digest`, `decision_signing_payload`, `verify_payload`,
//!     `validate_public_key`) plus server-address normalization. ONE Rust
//!     implementation of the security-critical bytes; no parallel Swift
//!     reimplementation to drift.
//!   - `runtime.rs` — the stateful async `NodClient` runtime (JSON-RPC in,
//!     `NodClientMessage` events out, `NodDeviceSigner` Secure Enclave callback).
//!
//! This must stay the ONLY UniFFI crate: two UniFFI xcframeworks collide on
//! `module.modulemap` in the `.xcodeproj` app build (that is why the former
//! `nod-proto-ffi` was merged in here). Extend this crate; never add a second.

uniffi::setup_scaffolding!();

mod runtime;
pub use runtime::{
    ClientFfiError, NodClient, NodClientObserver, NodDeviceKey, NodDeviceSigner,
    SignerCallbackError,
};

use nod_proto::{
    decision_signing_payload as proto_decision_signing_payload,
    request_digest as proto_request_digest, validate_public_key as proto_validate_public_key,
    verify_payload as proto_verify_payload, DecisionSigningInput, OptionKind, Request,
    SigningError,
};

// ---------------------------------------------------------------------------
// Decision-signing contract (from nod-proto).
//
// This is the **only** place the Swift app gets the request-digest /
// decision-payload byte construction and P-256 verification, so there is one
// Rust implementation of the security-critical path. The Secure Enclave still
// performs the actual signing in Swift; this decides *what* bytes get signed and
// verifies the result.
// ---------------------------------------------------------------------------

/// Errors surfaced across the FFI boundary to Swift.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SigningFfiError {
    #[error("invalid request json: {message}")]
    InvalidRequestJson { message: String },
    #[error("invalid signing public key")]
    InvalidPublicKey,
    #[error("invalid signature encoding")]
    InvalidSignatureEncoding,
    #[error("signature verification failed")]
    SignatureMismatch,
    #[error("signing failure: {message}")]
    Other { message: String },
}

impl From<SigningError> for SigningFfiError {
    fn from(err: SigningError) -> Self {
        match err {
            SigningError::Json(inner) => Self::InvalidRequestJson {
                message: inner.to_string(),
            },
            SigningError::InvalidPublicKey => Self::InvalidPublicKey,
            SigningError::InvalidSignatureEncoding => Self::InvalidSignatureEncoding,
            SigningError::SignatureMismatch => Self::SignatureMismatch,
            other => Self::Other {
                message: other.to_string(),
            },
        }
    }
}

/// Recompute the canonical request digest from the raw request JSON the server
/// sent, so the client can independently confirm it is approving the request it
/// saw rather than trusting the server-provided digest.
#[uniffi::export]
pub fn request_digest(request_json: String) -> Result<String, SigningFfiError> {
    let request: Request =
        serde_json::from_str(&request_json).map_err(|err| SigningFfiError::InvalidRequestJson {
            message: err.to_string(),
        })?;
    Ok(proto_request_digest(&request)?)
}

/// Apply the shared safe notification policy before native presentation.
#[uniffi::export]
pub fn notification_preview(request_json: String) -> Result<String, SigningFfiError> {
    let request: Request = serde_json::from_str(&request_json).map_err(|error| {
        SigningFfiError::InvalidRequestJson {
            message: error.to_string(),
        }
    })?;
    serde_json::to_string(&nod_proto::notification_preview(&request)).map_err(|error| {
        SigningFfiError::Other {
            message: error.to_string(),
        }
    })
}

/// Build the exact canonical string the client signs to resolve a request —
/// byte-for-byte identical to the server's verify path.
#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn decision_signing_payload(
    request_id: String,
    request_digest: String,
    option_id: String,
    option_kind: String,
    user_id: String,
    device_id: String,
    key_id: String,
    nonce: String,
    signed_at: String,
    text: Option<String>,
) -> String {
    let kind = OptionKind::from(option_kind.as_str());
    proto_decision_signing_payload(DecisionSigningInput {
        request_id: &request_id,
        request_digest: &request_digest,
        option_id: &option_id,
        option_kind: &kind,
        user_id: &user_id,
        device_id: &device_id,
        key_id: &key_id,
        nonce: &nonce,
        signed_at: &signed_at,
        text: text.as_deref(),
    })
}

/// Verify a base64url DER signature over `payload` with a base64url uncompressed
/// P-256 public key. Defense-in-depth: the client can confirm its own Secure
/// Enclave signature before sending it to the server.
#[uniffi::export]
pub fn verify_payload(
    public_key: String,
    payload: String,
    signature: String,
) -> Result<(), SigningFfiError> {
    Ok(proto_verify_payload(
        &public_key,
        payload.as_bytes(),
        &signature,
    )?)
}

/// Validate that a base64url string is a well-formed uncompressed P-256 key.
#[uniffi::export]
pub fn validate_public_key(public_key: String) -> Result<(), SigningFfiError> {
    Ok(proto_validate_public_key(&public_key)?)
}

// ---------------------------------------------------------------------------
// Server-address helpers (from nod-client-core).
// ---------------------------------------------------------------------------

/// Normalize a user-entered server address into a base URL (adds the scheme,
/// trims). Canonical logic shared with the TUI + desktop — replaces NodKit's
/// `NodServerAddress.normalizedBaseURL`.
#[uniffi::export]
pub fn normalize_base_url(value: String) -> String {
    nod_client_core::normalize_base_url(&value)
}

/// Stable per-server profile id derived from a base URL — replaces
/// `NodServerAddress.profileId`.
#[uniffi::export]
pub fn profile_id_for(base_url: String) -> String {
    nod_client_core::profile_id_for(&base_url)
}

/// Human-readable display name for a server base URL — replaces
/// `NodServerAddress.displayName`.
#[uniffi::export]
pub fn display_name_for(base_url: String) -> String {
    nod_client_core::display_name_for(&base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FFI layer must not alter the canonical decision payload: this is the
    /// same frozen vector nod-proto pins, reproduced through the exported entry
    /// point that Swift calls.
    #[test]
    fn ffi_decision_payload_matches_frozen_contract() {
        let payload = decision_signing_payload(
            "request-1".to_string(),
            "server-provided-request-digest".to_string(),
            "approve".to_string(),
            "approve".to_string(),
            "user-1".to_string(),
            "device-1".to_string(),
            "device-key-id".to_string(),
            "unique-device-nonce".to_string(),
            "2026-05-31T12:00:00.000Z".to_string(),
            Some("ship it".to_string()),
        );

        assert_eq!(
            payload,
            concat!(
                "nod-decision-v1\n",
                "request_id:request-1\n",
                "request_digest:server-provided-request-digest\n",
                "option_id:approve\n",
                "option_kind:approve\n",
                "user_id:user-1\n",
                "device_id:device-1\n",
                "key_id:device-key-id\n",
                "nonce:unique-device-nonce\n",
                "signed_at:2026-05-31T12:00:00.000Z\n",
                "text_sha256:bef4261f394bf71fd2b565cd76396ac9ed7953f9110c69ee49d7a82871238fbf\n"
            )
        );
    }

    #[test]
    fn ffi_request_digest_reads_request_json() {
        let json = r#"{
            "id": "request-1",
            "request_id": "request-1",
            "channel_id": "deployments",
            "recipients": ["owner"],
            "decision_resolution": "shared",
            "title": "Deploy?",
            "summary": "Production deploy",
            "body_markdown": "Approve deploy",
            "fields": [],
            "links": [],
            "image_url": null,
            "notification": { "redact": false, "title": null, "body": null },
            "dedupe_key": null,
            "expires_at": null,
            "status": "pending",
            "created_at": "2026-05-28T12:00:00.000Z",
            "updated_at": "2026-05-28T12:00:00.000Z",
            "resolved_at": null,
            "decision": null,
            "decisions": [],
            "callback_url": null,
            "options": []
        }"#;

        let digest = request_digest(json.to_string()).unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn ffi_request_digest_rejects_invalid_json() {
        let err = request_digest("not json".to_string()).unwrap_err();
        assert!(matches!(err, SigningFfiError::InvalidRequestJson { .. }));
    }

    #[test]
    fn ffi_validate_public_key_rejects_garbage() {
        assert!(matches!(
            validate_public_key("not-a-key".to_string()).unwrap_err(),
            SigningFfiError::InvalidPublicKey
        ));
    }

    /// Keep address normalization and credential profile identity identical
    /// across this boundary and NodKit's `NodServerAddress` vectors.
    #[test]
    fn matches_nodkit_server_address_vectors() {
        assert_eq!(
            normalize_base_url("nod.example.test".into()),
            "https://nod.example.test"
        );
        assert_eq!(
            normalize_base_url("localhost:8765".into()),
            "http://localhost:8765"
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8767".into()),
            "http://127.0.0.1:8767"
        );
        assert_eq!(
            profile_id_for("https://nod.example.test/team-a".into()),
            "server-b521bc3e42a2657d9057586a8760445eaf8e39437d05fa1278838956a7197da2"
        );
        assert_eq!(
            display_name_for("https://nod.example.test/team-a".into()),
            "nod.example.test/team-a"
        );
    }
}
