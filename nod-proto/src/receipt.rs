//! Deterministic, offline verification of a decision receipt against a pinned
//! device key.
//!
//! The server is not trusted here. Everything the verdict depends on is either
//! recomputed from the request the issuer received at create time or supplied
//! by the caller (the pinned keys and the clock). In particular the
//! `public_key` the server attaches to a signature record is ignored: a
//! compromised server could attach any key it likes to a forged receipt.
//!
//! What a passing verification establishes: a device holding the private half
//! of a pinned key signed exactly this option, on exactly this request content
//! (title, body, fields, links, options, recipients commitment, expiry …), with
//! this nonce and timestamp, and the server's recorded receipt is byte-for-byte
//! what the device signed. It does not establish that the nonce was unused
//! (replay is the server's job) or that the device key is still enrolled.

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use crate::{
    decision::{Decision, DecisionSignatureRecord},
    request::{OptionKind, Request, RequestOption},
    signing::{
        decision_signing_payload, request_digest, request_digest_v2, verify_payload,
        DecisionSigningInput, SigningError, PRIVATE_REQUEST_DIGEST_VERSION,
    },
    value_signing::{
        decision_signing_payload_v2, request_digest_v3, VALUE_DECISION_PAYLOAD_VERSION,
        VALUE_REQUEST_DIGEST_VERSION,
    },
    DECISION_SIGNING_ALGORITHM,
};

/// Default tolerance between the device's `signed_at` and the verifier's clock.
pub const DEFAULT_MAX_CLOCK_SKEW: Duration = Duration::minutes(10);

/// A device public key the caller trusts, recorded out of band (for example
/// from the admin devices list at enrollment time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedKey {
    pub user_id: String,
    /// When set, the receipt's `actor_device_id` must match it too.
    pub device_id: Option<String>,
    pub key_id: String,
    /// Base64url (no padding) uncompressed SEC1 P-256 point.
    pub public_key: String,
}

/// Inputs to [`verify_decision_receipt`].
#[derive(Debug, Clone)]
pub struct ReceiptVerificationInput<'a> {
    /// The request exactly as returned by the create response
    /// (`CreatedDecisionRequest.request`), i.e. the content the issuer asked for.
    pub request: &'a Request,
    /// The decision as returned by `/decision`, `/wait`, or the callback body.
    pub decision: &'a Decision,
    pub pinned_keys: &'a [PinnedKey],
    /// The verifier's clock.
    pub now: DateTime<Utc>,
    /// Tolerance for `signed_at` versus `now`. `None` uses
    /// [`DEFAULT_MAX_CLOCK_SKEW`].
    pub max_clock_skew: Option<Duration>,
    /// When true a `signed_at` outside the tolerance is an error instead of a
    /// warning.
    pub strict_time: bool,
}

/// Which request digest encoding the receipt was found to bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestVersion {
    /// `nod-request-v1`, the full-snapshot legacy digest.
    V1,
    /// `nod-request-v2`, the recipient-private digest.
    V2,
    /// `nod-request-v3`, the single-value digest.
    V3,
}

impl DigestVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => crate::signing::REQUEST_DIGEST_VERSION,
            Self::V2 => PRIVATE_REQUEST_DIGEST_VERSION,
            Self::V3 => VALUE_REQUEST_DIGEST_VERSION,
        }
    }
}

/// The facts a passing verification established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedReceipt {
    pub request_id: String,
    pub user_id: String,
    pub device_id: String,
    pub key_id: String,
    pub option_id: String,
    pub option_kind: OptionKind,
    pub request_digest: String,
    pub digest_version: DigestVersion,
    pub signed_at: DateTime<Utc>,
    /// Non-fatal observations, e.g. clock skew outside the tolerance when
    /// `strict_time` is off.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("decision carries no signature")]
    MissingSignature,
    #[error("unsupported signature algorithm {0:?}")]
    UnsupportedAlgorithm(String),
    #[error("unsupported signature payload_version {0:?}")]
    UnsupportedPayloadVersion(String),
    #[error("unsupported request signing version {0:?}")]
    UnsupportedDigestVersion(String),
    #[error("decision is for request {decision:?} but the created request is {request:?}")]
    RequestIdMismatch { request: String, decision: String },
    #[error("signature request_digest {found:?} matches none of the digests recomputed from the created request {expected:?}")]
    RequestDigestMismatch {
        expected: Vec<String>,
        found: String,
    },
    #[error("decision has no actor_user_id or actor_device_id")]
    MissingActor,
    #[error("no pinned key for user {user_id:?} with key_id {key_id:?}")]
    UnknownKey { user_id: String, key_id: String },
    #[error("pinned key for {key_id:?} belongs to device {pinned:?}, receipt names {found:?}")]
    DeviceMismatch {
        key_id: String,
        pinned: String,
        found: String,
    },
    #[error("value request receipt has no answer_value")]
    MissingAnswerValue,
    #[error("ordinary request receipt carries an answer_value")]
    UnexpectedAnswerValue,
    #[error("recorded signing_payload differs from the payload rebuilt from the decision")]
    SigningPayloadMismatch,
    #[error("signature does not verify under the pinned key: {0}")]
    SignatureInvalid(SigningError),
    #[error("option {0:?} is not one of the created request's options")]
    UnknownOption(String),
    #[error(
        "option {option_id:?} is kind {expected:?} in the created request, receipt says {found:?}"
    )]
    OptionKindMismatch {
        option_id: String,
        expected: OptionKind,
        found: OptionKind,
    },
    #[error("signed_at {0:?} is not an RFC 3339 timestamp")]
    InvalidSignedAt(String),
    #[error("signed_at {signed_at} is {skew} from now ({now}); tolerance is {tolerance}")]
    ClockSkew {
        signed_at: DateTime<Utc>,
        now: DateTime<Utc>,
        skew: Duration,
        tolerance: Duration,
    },
    #[error("could not recompute request digest: {0}")]
    Digest(SigningError),
    #[error("could not rebuild signing payload: {0}")]
    Payload(SigningError),
}

/// Verify a decision receipt against the created request and a pinned key set.
///
/// See the module documentation for what a pass means. The check order is:
/// signature presence and algorithm, request id, request digest recomputed
/// from `request`, option membership and kind, actor and pinned key lookup,
/// canonical payload equality, P-256 verification, then `signed_at`.
pub fn verify_decision_receipt(
    input: ReceiptVerificationInput<'_>,
) -> Result<VerifiedReceipt, ReceiptError> {
    let request = input.request;
    let decision = input.decision;
    let signature: &DecisionSignatureRecord = decision
        .signature
        .as_ref()
        .ok_or(ReceiptError::MissingSignature)?;
    if signature.algorithm != DECISION_SIGNING_ALGORITHM {
        return Err(ReceiptError::UnsupportedAlgorithm(
            signature.algorithm.clone(),
        ));
    }
    if decision.request_id != request.id
        || (!request.request_id.is_empty() && request.request_id != request.id)
    {
        return Err(ReceiptError::RequestIdMismatch {
            request: request.id.clone(),
            decision: decision.request_id.clone(),
        });
    }

    let (request_digest, digest_version) =
        match_request_digest(request, &signature.request_digest)?;
    let option = resolve_option(request, &decision.option_id)?;
    if option.kind != decision.option_kind {
        return Err(ReceiptError::OptionKindMismatch {
            option_id: decision.option_id.clone(),
            expected: option.kind.clone(),
            found: decision.option_kind.clone(),
        });
    }

    let (Some(user_id), Some(device_id)) = (
        decision.actor_user_id.as_deref(),
        decision.actor_device_id.as_deref(),
    ) else {
        return Err(ReceiptError::MissingActor);
    };
    let pinned = input
        .pinned_keys
        .iter()
        .find(|key| key.user_id == user_id && key.key_id == signature.key_id)
        .ok_or_else(|| ReceiptError::UnknownKey {
            user_id: user_id.to_string(),
            key_id: signature.key_id.clone(),
        })?;
    if let Some(pinned_device) = pinned.device_id.as_deref() {
        if pinned_device != device_id {
            return Err(ReceiptError::DeviceMismatch {
                key_id: signature.key_id.clone(),
                pinned: pinned_device.to_string(),
                found: device_id.to_string(),
            });
        }
    }

    let signed_at = DateTime::parse_from_rfc3339(&signature.signed_at)
        .map_err(|_| ReceiptError::InvalidSignedAt(signature.signed_at.clone()))?
        .with_timezone(&Utc);
    // The server canonicalises signed_at to millisecond UTC before building
    // the payload; mirror that so a non-canonical submission still rebuilds.
    let canonical_signed_at = signed_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let payload_input = DecisionSigningInput {
        request_id: &decision.request_id,
        request_digest: &request_digest,
        option_id: &decision.option_id,
        option_kind: &decision.option_kind,
        user_id,
        device_id,
        key_id: &signature.key_id,
        nonce: &signature.nonce,
        signed_at: &canonical_signed_at,
        text: decision.text.as_deref(),
    };
    let rebuilt = match signature.payload_version.as_deref() {
        None => {
            if decision.answer_value.is_some() || request.answer_field.is_some() {
                return Err(ReceiptError::UnexpectedAnswerValue);
            }
            decision_signing_payload(payload_input)
        }
        Some(VALUE_DECISION_PAYLOAD_VERSION) => {
            let answer = decision
                .answer_value
                .as_ref()
                .ok_or(ReceiptError::MissingAnswerValue)?;
            decision_signing_payload_v2(payload_input, answer).map_err(ReceiptError::Payload)?
        }
        Some(other) => {
            return Err(ReceiptError::UnsupportedPayloadVersion(other.to_string()));
        }
    };
    if rebuilt != signature.signing_payload {
        return Err(ReceiptError::SigningPayloadMismatch);
    }
    verify_payload(&pinned.public_key, rebuilt.as_bytes(), &signature.signature)
        .map_err(ReceiptError::SignatureInvalid)?;

    let mut warnings = Vec::new();
    let tolerance = input.max_clock_skew.unwrap_or(DEFAULT_MAX_CLOCK_SKEW);
    let skew = (input.now - signed_at).abs();
    if skew > tolerance {
        let error = ReceiptError::ClockSkew {
            signed_at,
            now: input.now,
            skew,
            tolerance,
        };
        if input.strict_time {
            return Err(error);
        }
        warnings.push(error.to_string());
    }
    if (decision.resolved_at - signed_at).abs() > tolerance {
        warnings.push(format!(
            "server resolved_at {} is {} from signed_at {}",
            decision.resolved_at,
            (decision.resolved_at - signed_at).abs(),
            signed_at
        ));
    }

    Ok(VerifiedReceipt {
        request_id: decision.request_id.clone(),
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        key_id: signature.key_id.clone(),
        option_id: decision.option_id.clone(),
        option_kind: decision.option_kind.clone(),
        request_digest,
        digest_version,
        signed_at,
        warnings,
    })
}

/// Recompute every digest the created request can legitimately be signed
/// under and require the receipt's digest to be one of them.
fn match_request_digest(
    request: &Request,
    found: &str,
) -> Result<(String, DigestVersion), ReceiptError> {
    let mut candidates: Vec<(String, DigestVersion)> = Vec::with_capacity(2);
    if request.answer_field.is_none() {
        candidates.push((
            request_digest(request).map_err(ReceiptError::Digest)?,
            DigestVersion::V1,
        ));
    }
    if let Some(signing) = request.signing.as_ref() {
        let commitment = signing.recipients_commitment.as_str();
        match signing.version.as_str() {
            PRIVATE_REQUEST_DIGEST_VERSION => candidates.push((
                request_digest_v2(request, commitment).map_err(ReceiptError::Digest)?,
                DigestVersion::V2,
            )),
            VALUE_REQUEST_DIGEST_VERSION => {
                let field = request
                    .answer_field
                    .as_ref()
                    .ok_or(ReceiptError::MissingAnswerValue)?;
                candidates.push((
                    request_digest_v3(request, commitment, field).map_err(ReceiptError::Digest)?,
                    DigestVersion::V3,
                ));
            }
            other => return Err(ReceiptError::UnsupportedDigestVersion(other.to_string())),
        }
    }
    candidates
        .iter()
        .find(|(digest, _)| digest == found)
        .cloned()
        .ok_or_else(|| ReceiptError::RequestDigestMismatch {
            expected: candidates.into_iter().map(|(digest, _)| digest).collect(),
            found: found.to_string(),
        })
}

/// The option a decision names, or the implicit `dismiss` an optionless
/// ordinary request accepts (mirrors the server's `implicit_dismiss_option`).
fn resolve_option(request: &Request, option_id: &str) -> Result<RequestOption, ReceiptError> {
    if let Some(option) = request.options.iter().find(|option| option.id == option_id) {
        return Ok(option.clone());
    }
    if request.options.is_empty() && request.answer_field.is_none() && option_id == "dismiss" {
        return Ok(RequestOption {
            id: "dismiss".to_string(),
            label: "Dismiss".to_string(),
            kind: OptionKind::Dismiss,
            style: "default".to_string(),
            requires_text: false,
            text_placeholder: None,
            destructive: false,
            foreground: false,
        });
    }
    Err(ReceiptError::UnknownOption(option_id.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::{
        request::{
            CardField, CardLink, DecisionResolution, RequestNotification, RequestSigningContext,
            RequestStatus,
        },
        signing::{generate_signing_key, recipients_commitment, sign_payload},
        AnswerField, AnswerValue,
    };

    const SALT: &str = "test-only-recipient-salt";

    fn created_request() -> Request {
        let mut request = Request {
            answer_field: None,
            id: "req-1".to_string(),
            request_id: "req-1".to_string(),
            channel_id: "deploys".to_string(),
            recipients: vec!["owner".to_string(), "platform".to_string()],
            decision_resolution: DecisionResolution::Shared,
            title: "Approve production deploy".to_string(),
            summary: "api-gateway v42".to_string(),
            body_markdown: "**Change:** roll forward.".to_string(),
            fields: vec![CardField {
                label: "Service".to_string(),
                value: "api-gateway".to_string(),
                style: None,
            }],
            links: vec![CardLink {
                label: "Diff".to_string(),
                url: "https://example.com/diff/42".to_string(),
            }],
            image_url: None,
            notification: RequestNotification::default(),
            dedupe_key: Some("deploy:42".to_string()),
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: None,
            options: vec![
                RequestOption {
                    id: "approve".to_string(),
                    label: "Approve".to_string(),
                    kind: OptionKind::Approve,
                    style: "default".to_string(),
                    requires_text: false,
                    text_placeholder: None,
                    destructive: false,
                    foreground: false,
                },
                RequestOption {
                    id: "reject".to_string(),
                    label: "Reject".to_string(),
                    kind: OptionKind::Reject,
                    style: "default".to_string(),
                    requires_text: false,
                    text_placeholder: None,
                    destructive: false,
                    foreground: false,
                },
            ],
            request_digest: None,
            signing: None,
        };
        stamp(&mut request);
        request
    }

    /// Mirror the server's create-response projection: v1 digest plus the
    /// salted v2/v3 signing context.
    fn stamp(request: &mut Request) {
        let commitment = recipients_commitment(&request.recipients, SALT).unwrap();
        if let Some(field) = request.answer_field.clone() {
            request.request_digest = None;
            request.signing = Some(RequestSigningContext {
                version: VALUE_REQUEST_DIGEST_VERSION.to_string(),
                recipients_commitment: commitment.clone(),
                request_digest: request_digest_v3(request, &commitment, &field).unwrap(),
            });
        } else {
            request.request_digest = Some(request_digest(request).unwrap());
            request.signing = Some(RequestSigningContext {
                version: PRIVATE_REQUEST_DIGEST_VERSION.to_string(),
                recipients_commitment: commitment.clone(),
                request_digest: request_digest_v2(request, &commitment).unwrap(),
            });
        }
    }

    struct Signer {
        private_key: String,
        pinned: PinnedKey,
    }

    fn new_signer(user_id: &str, device_id: &str, key_id: &str) -> Signer {
        let key = generate_signing_key();
        Signer {
            private_key: key.private_key,
            pinned: PinnedKey {
                user_id: user_id.to_string(),
                device_id: Some(device_id.to_string()),
                key_id: key_id.to_string(),
                public_key: key.public_key,
            },
        }
    }

    /// Produce the decision the server would record for a device signing
    /// `option_id` on `request` under `digest`, the way a client does.
    fn sign_decision(
        signer: &Signer,
        request: &Request,
        digest: &str,
        option_id: &str,
        text: Option<&str>,
        answer: Option<&AnswerValue>,
    ) -> Decision {
        let option = request
            .options
            .iter()
            .find(|option| option.id == option_id)
            .cloned()
            .unwrap_or_else(|| resolve_option(request, option_id).unwrap());
        let signed_at = "2026-05-28T12:05:00.000Z";
        let input = DecisionSigningInput {
            request_id: &request.id,
            request_digest: digest,
            option_id,
            option_kind: &option.kind,
            user_id: &signer.pinned.user_id,
            device_id: signer.pinned.device_id.as_deref().unwrap(),
            key_id: &signer.pinned.key_id,
            nonce: "nonce-1",
            signed_at,
            text,
        };
        let (payload, payload_version) = match answer {
            Some(answer) => (
                decision_signing_payload_v2(input, answer).unwrap(),
                Some(VALUE_DECISION_PAYLOAD_VERSION.to_string()),
            ),
            None => (decision_signing_payload(input), None),
        };
        let signature = sign_payload(&signer.private_key, payload.as_bytes()).unwrap();
        Decision {
            answer_value: answer.cloned(),
            request_id: request.id.clone(),
            option_id: option.id.clone(),
            option_kind: option.kind.clone(),
            option_label: option.label.clone(),
            text: text.map(ToOwned::to_owned),
            actor_user_id: Some(signer.pinned.user_id.clone()),
            actor_device_id: signer.pinned.device_id.clone(),
            signature: Some(DecisionSignatureRecord {
                payload_version,
                key_id: signer.pinned.key_id.clone(),
                algorithm: DECISION_SIGNING_ALGORITHM.to_string(),
                nonce: "nonce-1".to_string(),
                signed_at: signed_at.to_string(),
                request_digest: digest.to_string(),
                signing_payload: payload,
                signature,
                verified: true,
                public_key: Some(signer.pinned.public_key.clone()),
            }),
            resolved_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 5, 1).unwrap(),
        }
    }

    fn verify(
        request: &Request,
        decision: &Decision,
        keys: &[PinnedKey],
    ) -> Result<VerifiedReceipt, ReceiptError> {
        verify_decision_receipt(ReceiptVerificationInput {
            request,
            decision,
            pinned_keys: keys,
            now: Utc.with_ymd_and_hms(2026, 5, 28, 12, 6, 0).unwrap(),
            max_clock_skew: None,
            strict_time: false,
        })
    }

    #[test]
    fn verifies_a_v1_full_snapshot_receipt() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "approve", Some("ship it"), None);
        let verified = verify(&request, &decision, &[signer.pinned.clone()]).unwrap();
        assert_eq!(verified.user_id, "owner");
        assert_eq!(verified.device_id, "device-1");
        assert_eq!(verified.key_id, "key-1");
        assert_eq!(verified.option_id, "approve");
        assert_eq!(verified.option_kind, OptionKind::Approve);
        assert_eq!(verified.digest_version, DigestVersion::V1);
        assert!(verified.warnings.is_empty(), "{:?}", verified.warnings);
    }

    #[test]
    fn verifies_a_v2_private_projection_receipt() {
        let request = created_request();
        let signer = new_signer("platform", "device-2", "key-2");
        let digest = request.signing.as_ref().unwrap().request_digest.clone();
        let decision = sign_decision(&signer, &request, &digest, "reject", None, None);
        let verified = verify(&request, &decision, &[signer.pinned.clone()]).unwrap();
        assert_eq!(verified.digest_version, DigestVersion::V2);
        assert_eq!(verified.option_kind, OptionKind::Reject);
    }

    #[test]
    fn verifies_a_v3_value_receipt() {
        let mut request = created_request();
        request.answer_field = Some(AnswerField::Number {
            label: "Replicas".to_string(),
            default: AnswerValue::Number {
                decimal: "3".to_string(),
            },
        });
        stamp(&mut request);
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.signing.as_ref().unwrap().request_digest.clone();
        let answer = AnswerValue::Number {
            decimal: "5".to_string(),
        };
        let decision = sign_decision(&signer, &request, &digest, "approve", None, Some(&answer));
        let verified = verify(&request, &decision, &[signer.pinned.clone()]).unwrap();
        assert_eq!(verified.digest_version, DigestVersion::V3);

        let mut tampered = decision.clone();
        tampered.answer_value = Some(AnswerValue::Number {
            decimal: "50".to_string(),
        });
        assert!(matches!(
            verify(&request, &tampered, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::SigningPayloadMismatch
        ));
    }

    #[test]
    fn verifies_implicit_dismiss_on_optionless_request() {
        let mut request = created_request();
        request.options.clear();
        stamp(&mut request);
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "dismiss", None, None);
        let verified = verify(&request, &decision, &[signer.pinned.clone()]).unwrap();
        assert_eq!(verified.option_kind, OptionKind::Dismiss);
    }

    #[test]
    fn null_signature_is_a_failure() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let mut decision = sign_decision(&signer, &request, &digest, "approve", None, None);
        decision.signature = None;
        assert!(matches!(
            verify(&request, &decision, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::MissingSignature
        ));
    }

    #[test]
    fn wrong_pinned_key_fails_and_server_supplied_public_key_is_ignored() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "approve", None, None);

        // The record carries the real key, but only the pinned set counts.
        let mut other = new_signer("owner", "device-1", "key-1");
        other.pinned.public_key = generate_signing_key().public_key;
        assert!(matches!(
            verify(&request, &decision, &[other.pinned.clone()]).unwrap_err(),
            ReceiptError::SignatureInvalid(SigningError::SignatureMismatch)
        ));

        // Conversely a forged record pointing at an attacker key never passes
        // when the pinned key is the real one.
        let attacker = new_signer("owner", "device-1", "key-1");
        let mut forged = sign_decision(&attacker, &request, &digest, "approve", None, None);
        forged.signature.as_mut().unwrap().public_key = Some(attacker.pinned.public_key.clone());
        assert!(matches!(
            verify(&request, &forged, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::SignatureInvalid(SigningError::SignatureMismatch)
        ));

        // Unknown (user, key_id) is reported as such.
        let unrelated = new_signer("someone-else", "device-9", "key-9");
        assert!(matches!(
            verify(&request, &decision, &[unrelated.pinned.clone()]).unwrap_err(),
            ReceiptError::UnknownKey { .. }
        ));

        // A pinned device id that disagrees with the receipt fails too.
        let mut wrong_device = signer.pinned.clone();
        wrong_device.device_id = Some("device-other".to_string());
        assert!(matches!(
            verify(&request, &decision, &[wrong_device]).unwrap_err(),
            ReceiptError::DeviceMismatch { .. }
        ));
    }

    #[test]
    fn tampered_title_fails_the_digest_check() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "approve", None, None);
        let mut shown = request.clone();
        shown.title = "Approve production deploy (and delete the database)".to_string();
        assert!(matches!(
            verify(&shown, &decision, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::RequestDigestMismatch { .. }
        ));
    }

    #[test]
    fn tampered_option_fails() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "reject", None, None);

        // Server rewrites the recorded option to "approve" but cannot re-sign.
        let mut flipped = decision.clone();
        flipped.option_id = "approve".to_string();
        flipped.option_kind = OptionKind::Approve;
        assert!(matches!(
            verify(&request, &flipped, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::SigningPayloadMismatch
        ));

        // Server rewrites the recorded kind only.
        let mut kind_only = decision.clone();
        kind_only.option_kind = OptionKind::Approve;
        assert!(matches!(
            verify(&request, &kind_only, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::OptionKindMismatch { .. }
        ));

        // Option that was never offered.
        let mut unknown = decision.clone();
        unknown.option_id = "deploy_everything".to_string();
        assert!(matches!(
            verify(&request, &unknown, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::UnknownOption(_)
        ));
    }

    #[test]
    fn tampered_text_and_payload_fail() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "approve", Some("yes"), None);

        let mut text = decision.clone();
        text.text = Some("yes, and also rm -rf".to_string());
        assert!(matches!(
            verify(&request, &text, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::SigningPayloadMismatch
        ));

        // A payload that matches the fields but was never signed.
        let mut unsigned = decision.clone();
        unsigned.signature.as_mut().unwrap().signature = sign_payload(
            &generate_signing_key().private_key,
            decision
                .signature
                .as_ref()
                .unwrap()
                .signing_payload
                .as_bytes(),
        )
        .unwrap();
        assert!(matches!(
            verify(&request, &unsigned, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::SignatureInvalid(_)
        ));

        let mut algorithm = decision.clone();
        algorithm.signature.as_mut().unwrap().algorithm = "ed25519".to_string();
        assert!(matches!(
            verify(&request, &algorithm, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::UnsupportedAlgorithm(_)
        ));

        let mut other_request = decision.clone();
        other_request.request_id = "req-2".to_string();
        assert!(matches!(
            verify(&request, &other_request, &[signer.pinned.clone()]).unwrap_err(),
            ReceiptError::RequestIdMismatch { .. }
        ));
    }

    #[test]
    fn clock_skew_warns_by_default_and_fails_when_strict() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let decision = sign_decision(&signer, &request, &digest, "approve", None, None);
        let later = Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0).unwrap();
        let lenient = verify_decision_receipt(ReceiptVerificationInput {
            request: &request,
            decision: &decision,
            pinned_keys: &[signer.pinned.clone()],
            now: later,
            max_clock_skew: None,
            strict_time: false,
        })
        .unwrap();
        assert_eq!(lenient.warnings.len(), 1, "{:?}", lenient.warnings);
        assert!(matches!(
            verify_decision_receipt(ReceiptVerificationInput {
                request: &request,
                decision: &decision,
                pinned_keys: &[signer.pinned.clone()],
                now: later,
                max_clock_skew: Some(Duration::hours(1)),
                strict_time: true,
            })
            .unwrap_err(),
            ReceiptError::ClockSkew { .. }
        ));
    }

    #[test]
    fn non_canonical_signed_at_still_rebuilds_the_canonical_payload() {
        let request = created_request();
        let signer = new_signer("owner", "device-1", "key-1");
        let digest = request.request_digest.clone().unwrap();
        let mut decision = sign_decision(&signer, &request, &digest, "approve", None, None);
        // Same instant, different spelling: the server records what the client
        // sent but signs over the millisecond-UTC form.
        decision.signature.as_mut().unwrap().signed_at = "2026-05-28T14:05:00+02:00".to_string();
        verify(&request, &decision, &[signer.pinned.clone()]).unwrap();
    }
}
