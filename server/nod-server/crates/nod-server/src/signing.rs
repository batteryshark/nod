use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    error::ApiError,
    models::{DecisionRequest, Device, RequestOption, SubmitDecisionSignature},
};

/// The only decision-signing algorithm Nod accepts. Mirrors
/// [`nod_proto::DECISION_SIGNING_ALGORITHM`].
pub const DEFAULT_ALGORITHM: &str = nod_proto::DECISION_SIGNING_ALGORITHM;

/// Verify a submitted decision signature against the request snapshot and the
/// device's registered key.
///
/// The canonical digest, signing payload, and P-256 verification all come from
/// `nod-proto` (the single source of truth shared with clients). This function
/// keeps the server-side policy: algorithm, key-id, nonce, and digest-binding
/// checks. Returns the canonical `(request_digest, signing_payload)`.
pub fn verify_decision_signature(
    request: &DecisionRequest,
    option: &RequestOption,
    actor: &Device,
    text: Option<&str>,
    provided: &SubmitDecisionSignature,
) -> Result<(String, String), ApiError> {
    let Some(device_key_id) = actor.signing_key_id.as_deref() else {
        return Err(ApiError::BadRequest(
            "device does not have a registered signing key".to_string(),
        ));
    };
    let Some(public_key) = actor.signing_public_key.as_deref() else {
        return Err(ApiError::BadRequest(
            "device does not have a registered signing key".to_string(),
        ));
    };
    let algorithm = actor
        .signing_key_algorithm
        .as_deref()
        .unwrap_or(DEFAULT_ALGORITHM);
    if algorithm != DEFAULT_ALGORITHM || provided.algorithm != DEFAULT_ALGORITHM {
        return Err(ApiError::BadRequest(
            "unsupported decision signature algorithm".to_string(),
        ));
    }
    if provided.key_id != device_key_id {
        return Err(ApiError::BadRequest(
            "decision signature key_id does not match this device".to_string(),
        ));
    }
    if provided.nonce.trim().is_empty() || provided.nonce.len() > 160 {
        return Err(ApiError::BadRequest(
            "decision signature nonce is required".to_string(),
        ));
    }

    // to_wire honors a stamped canonical digest, so even a per-user projection
    // reaching this path would verify against the full-snapshot digest.
    let wire = request.to_wire();
    let legacy_digest = wire
        .request_digest
        .ok_or_else(|| ApiError::Internal("could not compute request digest".to_string()))?;
    let private_digest = wire
        .signing
        .as_ref()
        .map(|context| context.request_digest.as_str());
    if provided.request_digest != legacy_digest
        && Some(provided.request_digest.as_str()) != private_digest
    {
        return Err(ApiError::BadRequest(
            "decision signature request_digest does not match the request snapshot".to_string(),
        ));
    }
    let request_digest = provided.request_digest.clone();

    let signed_at = DateTime::parse_from_rfc3339(&provided.signed_at)
        .map_err(|_| ApiError::BadRequest("decision signature signed_at is invalid".to_string()))?
        .with_timezone(&Utc);
    let signed_at = signed_at.to_rfc3339_opts(SecondsFormat::Millis, true);

    let signing_payload = nod_proto::decision_signing_payload(nod_proto::DecisionSigningInput {
        request_id: &request.id,
        request_digest: &request_digest,
        option_id: &option.id,
        option_kind: &option.kind,
        user_id: &actor.user_id,
        device_id: &actor.id,
        key_id: &provided.key_id,
        nonce: &provided.nonce,
        signed_at: &signed_at,
        text,
    });

    nod_proto::verify_payload(public_key, signing_payload.as_bytes(), &provided.signature)
        .map_err(|_| ApiError::Forbidden)?;
    Ok((request_digest, signing_payload))
}

/// Validate that a device's registered signing public key is a well-formed
/// uncompressed P-256 key.
pub fn validate_device_public_key(public_key: &str) -> Result<(), ApiError> {
    nod_proto::validate_public_key(public_key)
        .map_err(|_| ApiError::BadRequest("registered signing public key is invalid".to_string()))
}
