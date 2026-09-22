//! Decision value types: a recorded decision and the signatures that attest to it.
//!
//! Two signature shapes exist by design:
//! - [`DecisionSignature`] is what a client *submits* alongside its decision.
//! - [`DecisionSignatureRecord`] is what the server *records* and republishes,
//!   adding its verification verdict and the canonical payload it reconstructed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::{request::OptionKind, DECISION_SIGNING_ALGORITHM};

/// The signature a client submits when resolving a request.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSignature {
    pub key_id: String,
    #[serde(default = "default_signature_algorithm")]
    pub algorithm: String,
    pub nonce: String,
    pub signed_at: String,
    pub request_digest: String,
    pub signature: String,
}

/// The signature record the server stores and republishes on a decision. Adds
/// the server's verification verdict (`verified`) and the canonical
/// `signing_payload` it reconstructed during verification.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSignatureRecord {
    pub key_id: String,
    pub algorithm: String,
    pub nonce: String,
    pub signed_at: String,
    pub request_digest: String,
    pub signing_payload: String,
    pub signature: String,
    pub verified: bool,
    /// Retained with the receipt so device revocation cannot erase its verifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// A recorded decision on a request. Carries the optional signature record so
/// clients can display and independently verify it (the previous client model
/// silently dropped it).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub request_id: String,
    pub option_id: String,
    pub option_kind: OptionKind,
    pub option_label: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub actor_user_id: Option<String>,
    #[serde(default)]
    pub actor_device_id: Option<String>,
    #[serde(default)]
    pub signature: Option<DecisionSignatureRecord>,
    #[typeshare(serialized_as = "String")]
    pub resolved_at: DateTime<Utc>,
}

/// One user's decision under per-user resolution.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDecision {
    pub user_id: String,
    pub decision: Decision,
}

/// The body a client POSTs to resolve a request. Strict (`deny_unknown_fields`)
/// because it is client-supplied input.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitDecisionRequest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub signature: Option<DecisionSignature>,
}

fn default_signature_algorithm() -> String {
    DECISION_SIGNING_ALGORITHM.to_string()
}
