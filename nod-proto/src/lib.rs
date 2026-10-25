//! `nod-proto` — the canonical wire protocol and decision-signing contract for Nod.
//!
//! This crate is the single source of truth for the request/decision wire types
//! and the canonical signing contract: `nod-request-v1`/`nod-decision-v1` for
//! ordinary cards and `nod-request-v2`/`nod-decision-v2` for value cards. Older
//! encoders stay frozen for verification only.
//! The server and every Rust client depend on it directly; Swift and TypeScript
//! types are generated from it (typeshare) and the signing crypto is shared into
//! the Apple clients via UniFFI, so there is exactly one implementation of the
//! security-critical path. See `ARCHITECTURE_NOTES.md` (repo root) §2 for the
//! decision record.

/// The only decision-signing algorithm Nod supports: ECDSA over P-256 with
/// SHA-256 and ASN.1/DER signatures. Lives here because the wire DTOs default to
/// it and the canonical signing crypto is built around it.
pub const DECISION_SIGNING_ALGORITHM: &str = "p256_ecdsa_sha256";

pub mod answer;
pub mod attestation;
pub mod decision;
pub mod notification;
mod preferences;
mod preview;
pub mod receipt;
pub mod request;
pub mod signing;
pub mod value_signing;

pub use answer::{AnswerChoice, AnswerField, AnswerValue};
pub use attestation::{DeviceAttestationStatus, DeviceAttestationSummary};
pub use decision::{
    Decision, DecisionSignature, DecisionSignatureRecord, SubmitDecisionRequest, UserDecision,
};
pub use notification::{NotificationDelivery, NotificationDeliveryMode};
pub use preferences::DeviceNotificationPreferences;
pub use preview::{notification_preview, NotificationPreview};
pub use receipt::{
    verify_decision_receipt, DigestVersion, PinnedKey, ReceiptError, ReceiptVerificationInput,
    VerifiedReceipt, DEFAULT_MAX_CLOCK_SKEW,
};
pub use request::{
    CardField, CardLink, CreateDecisionRequest, CreatedDecisionRequest, DecisionResolution,
    OptionKind, Request, RequestNotification, RequestOption, RequestSigningContext, RequestStatus,
};
#[allow(deprecated)]
pub use signing::{
    decision_signing_payload, generate_signing_key, is_value_request, public_key_for,
    recipients_commitment, request_digest, request_digest_for_version,
    request_digest_legacy_line_snapshot, request_digest_legacy_structured_v2, request_digest_v1,
    request_digest_v2, sign_payload, validate_public_key, verify_payload, DecisionSigningInput,
    SigningError, SigningKeyPair, DECISION_PAYLOAD_VERSION,
    LEGACY_LINE_SNAPSHOT_REQUEST_DIGEST_VERSION, LEGACY_STRUCTURED_REQUEST_DIGEST_VERSION,
    PRIVATE_REQUEST_DIGEST_VERSION, REQUEST_DIGEST_VERSION, UNCOMPRESSED_P256_PUBLIC_KEY_LENGTH,
    UNCOMPRESSED_P256_PUBLIC_KEY_PREFIX,
};
