//! Canonical request digest, decision signing payload, and the P-256 signing
//! crypto — the byte-exact contract that clients sign and the server verifies.
//!
//! Previously this lived twice and across two crypto stacks (the server verified
//! with `ring`, clients signed with `p256`). It is now the single channel of
//! truth, standardized on pure-Rust `p256` so it cross-compiles cleanly into the
//! Apple clients via UniFFI.
//!
//! Every formatting choice in the canonical strings is load-bearing: millisecond
//! RFC3339 with a `Z` suffix, field order, the `\n` joins, the version tags, and
//! the serde-JSON encoding of `fields`/`links`/`options`. Changing any of it
//! changes existing signatures and invalidates the append-only audit log — so
//! changes must be caught by the protocol-freeze vectors (see the tests below).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use p256::ecdsa::{
    signature::{Signer, Verifier},
    Signature, SigningKey, VerifyingKey,
};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

use crate::request::{OptionKind, Request};

/// Version tag prefixing the request digest snapshot.
pub const REQUEST_DIGEST_VERSION: &str = "nod-request-v1";
pub const PRIVATE_REQUEST_DIGEST_VERSION: &str = "nod-request-v2";
/// Version tag prefixing the decision signing payload.
pub const DECISION_PAYLOAD_VERSION: &str = "nod-decision-v1";

/// Length of an uncompressed SEC1 P-256 public key (`0x04 || X || Y`).
pub const UNCOMPRESSED_P256_PUBLIC_KEY_LENGTH: usize = 65;
/// Leading byte of an uncompressed SEC1 point.
pub const UNCOMPRESSED_P256_PUBLIC_KEY_PREFIX: u8 = 4;

/// Errors from canonicalization or P-256 signing/verification.
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("request serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid signing private key")]
    InvalidPrivateKey,
    #[error("invalid signing public key")]
    InvalidPublicKey,
    #[error("invalid signature encoding")]
    InvalidSignatureEncoding,
    #[error("signature verification failed")]
    SignatureMismatch,
    #[error("invalid recipient commitment")]
    InvalidRecipientCommitment,
}

/// A freshly generated P-256 signing key, both halves base64url (no padding).
pub struct SigningKeyPair {
    pub private_key: String,
    pub public_key: String,
}

/// Inputs to [`decision_signing_payload`]. Deliberately primitive (no `Device`
/// type) so the one function serves the server's verify path and every client's
/// sign path, including the UniFFI-bound Apple clients.
pub struct DecisionSigningInput<'a> {
    pub request_id: &'a str,
    pub request_digest: &'a str,
    pub option_id: &'a str,
    pub option_kind: &'a OptionKind,
    pub user_id: &'a str,
    pub device_id: &'a str,
    pub key_id: &'a str,
    pub nonce: &'a str,
    pub signed_at: &'a str,
    pub text: Option<&'a str>,
}

/// The canonical string a client signs to resolve a request. Field names,
/// order, and the trailing newline are part of the signing contract.
pub fn decision_signing_payload(input: DecisionSigningInput<'_>) -> String {
    [
        DECISION_PAYLOAD_VERSION.to_string(),
        format!("request_id:{}", input.request_id),
        format!("request_digest:{}", input.request_digest),
        format!("option_id:{}", input.option_id),
        format!("option_kind:{}", input.option_kind.as_str()),
        format!("user_id:{}", input.user_id),
        format!("device_id:{}", input.device_id),
        format!("key_id:{}", input.key_id),
        format!("nonce:{}", input.nonce),
        format!("signed_at:{}", input.signed_at),
        format!(
            "text_sha256:{}",
            sha256_hex(input.text.unwrap_or("").as_bytes())
        ),
        String::new(),
    ]
    .join("\n")
}

/// The digest over the immutable request snapshot (not delivery state or later
/// decisions). This is what a client's signature binds to, and what the client
/// can independently recompute to confirm it is approving the request it saw.
///
/// The snapshot includes the full recipient list, while per-user views filter
/// recipients down to the viewing user — so for multi-recipient requests the
/// digest a device view carries is server-attested (stamped from the full
/// snapshot before projection) rather than recomputable from the view alone.
pub fn request_digest(request: &Request) -> Result<String, SigningError> {
    let options = request
        .options
        .iter()
        .map(|option| {
            serde_json::json!({
                "id": option.id,
                "label": option.label,
                "kind": option.kind.as_str(),
                "style": option.style,
                "requires_text": option.requires_text,
                "text_placeholder": option.text_placeholder,
                "destructive": option.destructive,
                "foreground": option.foreground,
            })
        })
        .collect::<Vec<_>>();
    let fields = serde_json::to_string(&request.fields)?;
    let links = serde_json::to_string(&request.links)?;
    let options = serde_json::to_string(&options)?;
    let snapshot = format!(
        concat!(
            "nod-request-v1\n",
            "request_id:{request_id}\n",
            "channel_id:{channel_id}\n",
            "recipients:{recipients}\n",
            "decision_resolution:{decision_resolution}\n",
            "title_sha256:{title}\n",
            "summary_sha256:{summary}\n",
            "body_markdown_sha256:{body}\n",
            "fields_sha256:{fields}\n",
            "links_sha256:{links}\n",
            "image_url:{image_url}\n",
            "dedupe_key:{dedupe_key}\n",
            "expires_at:{expires_at}\n",
            "created_at:{created_at}\n",
            "options_sha256:{options}\n"
        ),
        request_id = request.id,
        channel_id = request.channel_id,
        recipients = request.recipients.join(","),
        decision_resolution = request.decision_resolution.as_str(),
        title = sha256_hex(request.title.as_bytes()),
        summary = sha256_hex(request.summary.as_bytes()),
        body = sha256_hex(request.body_markdown.as_bytes()),
        fields = sha256_hex(fields.as_bytes()),
        links = sha256_hex(links.as_bytes()),
        image_url = request.image_url.as_deref().unwrap_or(""),
        dedupe_key = request.dedupe_key.as_deref().unwrap_or(""),
        expires_at = request
            .expires_at
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_default(),
        created_at = request
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        options = sha256_hex(options.as_bytes()),
    );
    Ok(sha256_hex(snapshot.as_bytes()))
}

/// Salt remains on the server; exposing it would make small recipient lists
/// guessable. Preserve it with the original request for evidence export.
pub fn recipients_commitment(recipients: &[String], salt: &str) -> Result<String, SigningError> {
    let encoded = serde_json::to_string(&(salt, recipients))?;
    Ok(sha256_hex(
        format!("nod-recipients-v2\n{encoded}").as_bytes(),
    ))
}

/// Recompute the content digest of a private projection. The visible recipient
/// list is a delivery view; only the immutable commitment participates in v2.
/// The v1 content encoding stays frozen for historical signature verification.
pub fn request_digest_v2(request: &Request, commitment: &str) -> Result<String, SigningError> {
    if commitment.len() != 64 || !commitment.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SigningError::InvalidRecipientCommitment);
    }
    // Structured encoding keeps free-form URLs and dedupe keys from changing
    // field boundaries. Delivery state and recipient projections are excluded.
    let content = serde_json::json!({
        "request_id": request.id,
        "channel_id": request.channel_id,
        "recipients_commitment": commitment,
        "decision_resolution": request.decision_resolution,
        "title": request.title,
        "summary": request.summary,
        "body_markdown": request.body_markdown,
        "fields": request.fields,
        "links": request.links,
        "image_url": request.image_url,
        "notification": request.notification,
        "dedupe_key": request.dedupe_key,
        "expires_at": request.expires_at.map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        "created_at": request.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "callback_url": request.callback_url,
        "options": request.options,
    });
    Ok(sha256_hex(
        format!(
            "{PRIVATE_REQUEST_DIGEST_VERSION}\n{}\n",
            serde_json::to_string(&content)?
        )
        .as_bytes(),
    ))
}

// --- P-256 signing crypto ----------------------------------------------------

/// Generate a new P-256 software signing key. Clients backed by hardware keys
/// (e.g. Secure Enclave) supply their own public key and only need
/// [`verify_payload`]/[`decision_signing_payload`].
pub fn generate_signing_key() -> SigningKeyPair {
    let signing_key = SigningKey::random(&mut OsRng);
    SigningKeyPair {
        private_key: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
        public_key: encode_public_key(&signing_key),
    }
}

/// Derive the uncompressed base64url public key for a base64url private key.
pub fn public_key_for(private_key_b64: &str) -> Result<String, SigningError> {
    Ok(encode_public_key(&decode_signing_key(private_key_b64)?))
}

/// Sign a canonical payload, returning a base64url DER signature.
pub fn sign_payload(private_key_b64: &str, payload: &[u8]) -> Result<String, SigningError> {
    let signing_key = decode_signing_key(private_key_b64)?;
    let signature: Signature = signing_key.sign(payload);
    Ok(URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()))
}

/// Verify a base64url DER signature over a payload with a base64url uncompressed
/// public key. `from_sec1_bytes` performs full point validation (rejecting
/// off-curve keys), which is stricter than the previous `ring`-based path.
pub fn verify_payload(
    public_key_b64: &str,
    payload: &[u8],
    signature_b64: &str,
) -> Result<(), SigningError> {
    let public_key = decode_public_key(public_key_b64)?;
    let verifying_key =
        VerifyingKey::from_sec1_bytes(&public_key).map_err(|_| SigningError::InvalidPublicKey)?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| SigningError::InvalidSignatureEncoding)?;
    let signature = Signature::from_der(&signature_bytes)
        .map_err(|_| SigningError::InvalidSignatureEncoding)?;
    verifying_key
        .verify(payload, &signature)
        .map_err(|_| SigningError::SignatureMismatch)
}

/// Validate that a base64url string is a well-formed uncompressed P-256 key.
pub fn validate_public_key(public_key_b64: &str) -> Result<(), SigningError> {
    decode_public_key(public_key_b64).map(|_| ())
}

fn encode_public_key(signing_key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(
        signing_key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes(),
    )
}

fn decode_signing_key(private_key_b64: &str) -> Result<SigningKey, SigningError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(private_key_b64)
        .map_err(|_| SigningError::InvalidPrivateKey)?;
    SigningKey::from_slice(&bytes).map_err(|_| SigningError::InvalidPrivateKey)
}

fn decode_public_key(public_key_b64: &str) -> Result<Vec<u8>, SigningError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(public_key_b64)
        .map_err(|_| SigningError::InvalidPublicKey)?;
    match bytes.as_slice() {
        [UNCOMPRESSED_P256_PUBLIC_KEY_PREFIX, ..]
            if bytes.len() == UNCOMPRESSED_P256_PUBLIC_KEY_LENGTH =>
        {
            Ok(bytes)
        }
        _ => Err(SigningError::InvalidPublicKey),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::request::{
        CardField, CardLink, DecisionResolution, OptionKind, Request, RequestNotification,
        RequestOption, RequestStatus,
    };

    /// Frozen protocol vector: this exact payload was the signing contract the
    /// server and client independently reproduced before centralization. If it
    /// ever changes, existing decision signatures and audit records stop
    /// verifying — that must be a conscious, versioned protocol change.
    #[test]
    fn decision_payload_matches_frozen_contract() {
        let payload = decision_signing_payload(DecisionSigningInput {
            request_id: "request-1",
            request_digest: "server-provided-request-digest",
            option_id: "approve",
            option_kind: &OptionKind::Approve,
            user_id: "user-1",
            device_id: "device-1",
            key_id: "device-key-id",
            nonce: "unique-device-nonce",
            signed_at: "2026-05-31T12:00:00.000Z",
            text: Some("ship it"),
        });

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

    /// Frozen protocol vector for the request digest. Pins the canonicalization
    /// (field order, the sha256s of title/summary/body/fields/links/options, and
    /// the millisecond RFC3339 timestamps) so a refactor that changes the digest
    /// is caught here instead of silently invalidating every signature already
    /// written to the append-only audit log.
    #[test]
    fn request_digest_matches_frozen_contract() {
        assert_eq!(
            request_digest(&canonical_request()).unwrap(),
            "97e2edc559c47570f31c154f535ee6b019a509e998bf8a0c7f7b1cccb75d1f3f"
        );
    }

    #[test]
    fn private_digest_verifies_projected_content_without_recipients() {
        let mut request = canonical_request();
        let commitment = recipients_commitment(&request.recipients, "fixed-test-salt").unwrap();
        let expected = request_digest_v2(&request, &commitment).unwrap();
        // Independently calculated from the canonical JSON with Python's
        // hashlib/json; freezes v2 without changing the original v1 vector.
        assert_eq!(
            expected,
            "fdcd92dc9e5001b0cf171bdaf2f7d013f5898b2605c3d829f3e0f75aa2523dc3"
        );
        request.recipients = vec!["owner".to_string()];
        assert_eq!(request_digest_v2(&request, &commitment).unwrap(), expected);
        request.title.push_str(" changed");
        assert_ne!(request_digest_v2(&request, &commitment).unwrap(), expected);
    }

    #[test]
    fn recipient_commitment_binds_recipient_list_and_salt() {
        let recipients = canonical_request().recipients;
        let original = recipients_commitment(&recipients, "salt-a").unwrap();
        assert_ne!(
            original,
            recipients_commitment(&recipients, "salt-b").unwrap()
        );
        assert_ne!(
            original,
            recipients_commitment(&recipients[..1], "salt-a").unwrap()
        );
    }

    fn canonical_request() -> Request {
        Request {
            id: "request-1".to_string(),
            request_id: "request-1".to_string(),
            channel_id: "deployments".to_string(),
            recipients: vec!["owner".to_string(), "platform".to_string()],
            decision_resolution: DecisionResolution::Shared,
            title: "Approve production deploy".to_string(),
            summary: "api-gateway v42 is ready".to_string(),
            body_markdown: "**Change:** roll forward api-gateway to v42.".to_string(),
            fields: vec![CardField {
                label: "Service".to_string(),
                value: "api-gateway".to_string(),
                style: None,
            }],
            links: vec![CardLink {
                label: "Diff".to_string(),
                url: "https://example.com/diff/42".to_string(),
            }],
            image_url: Some("https://example.com/canary.png".to_string()),
            notification: RequestNotification::default(),
            dedupe_key: Some("deploy:api-gateway:42".to_string()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2027-01-01T00:10:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            status: RequestStatus::Pending,
            created_at: DateTime::parse_from_rfc3339("2026-05-28T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-05-28T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: Some("https://agent.example.com/nod/callback".to_string()),
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
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let keys = generate_signing_key();
        let payload = b"nod-decision-v1\nrequest_id:r\n";
        let signature = sign_payload(&keys.private_key, payload).unwrap();
        verify_payload(&keys.public_key, payload, &signature).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let keys = generate_signing_key();
        let signature = sign_payload(&keys.private_key, b"original").unwrap();
        assert!(matches!(
            verify_payload(&keys.public_key, b"tampered", &signature).unwrap_err(),
            SigningError::SignatureMismatch
        ));
    }

    #[test]
    fn verify_rejects_another_keys_signature() {
        let a = generate_signing_key();
        let b = generate_signing_key();
        let signature = sign_payload(&a.private_key, b"msg").unwrap();
        assert!(matches!(
            verify_payload(&b.public_key, b"msg", &signature).unwrap_err(),
            SigningError::SignatureMismatch
        ));
    }

    #[test]
    fn public_key_for_matches_generated_pair() {
        let keys = generate_signing_key();
        assert_eq!(public_key_for(&keys.private_key).unwrap(), keys.public_key);
    }

    #[test]
    fn generated_public_key_is_uncompressed_p256() {
        let keys = generate_signing_key();
        validate_public_key(&keys.public_key).unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(keys.public_key).unwrap();
        assert_eq!(bytes.len(), UNCOMPRESSED_P256_PUBLIC_KEY_LENGTH);
        assert_eq!(bytes[0], UNCOMPRESSED_P256_PUBLIC_KEY_PREFIX);
    }
}
