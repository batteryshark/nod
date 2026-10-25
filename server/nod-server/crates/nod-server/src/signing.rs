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
pub fn verify_value_decision_signature(
    request: &DecisionRequest,
    option: &RequestOption,
    actor: &Device,
    text: Option<&str>,
    provided: &SubmitDecisionSignature,
    answer: Option<&nod_proto::AnswerValue>,
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

    // A card does not persist the request-digest version it was issued under;
    // the read path re-derives the current one. Acceptance therefore checks the
    // submitted digest against every encoder that could have issued one for
    // this card: dispatch on the answer field, and on the decision payload tag
    // stored with the signature (nod-decision-v2 selects the value family).
    let wire = request.to_wire();
    let mut candidates: Vec<String> = Vec::new();
    if let Some(field) = &request.answer_field {
        let commitment = wire
            .signing
            .as_ref()
            .map(|context| context.recipients_commitment.as_str())
            .ok_or_else(|| {
                ApiError::BadRequest("value request is missing its signing context".to_string())
            })?;
        candidates.push(
            nod_proto::value_signing::value_request_digest(&wire, commitment, field)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?,
        );
        candidates.push(
            nod_proto::value_signing::value_request_digest_legacy_v3(&wire, commitment, field)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?,
        );
    } else {
        // Prefer the digest actually stamped on the wire (the v1.1.0 shape),
        // then fall back to recomputing the same encoder.
        let legacy = match wire.request_digest.clone() {
            Some(stamped) => stamped,
            None => nod_proto::request_digest_legacy_line_snapshot(&wire)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?,
        };
        candidates.push(legacy);
        if let Some(context) = &wire.signing {
            candidates.push(
                nod_proto::request_digest_v1(&wire, &context.recipients_commitment)
                    .map_err(|err| ApiError::BadRequest(err.to_string()))?,
            );
            candidates.push(
                nod_proto::request_digest_legacy_structured_v2(
                    &wire,
                    &context.recipients_commitment,
                )
                .map_err(|err| ApiError::BadRequest(err.to_string()))?,
            );
        }
    }
    if !candidates
        .iter()
        .any(|candidate| candidate == &provided.request_digest)
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

    let input = nod_proto::DecisionSigningInput {
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
    };
    let signing_payload = if let Some(field) = &request.answer_field {
        if provided.payload_version.as_deref()
            != Some(nod_proto::value_signing::VALUE_DECISION_PAYLOAD_VERSION)
            || provided.signed_at != signed_at
        {
            return Err(ApiError::BadRequest(
                "invalid value signature context".into(),
            ));
        }
        let answer =
            answer.ok_or_else(|| ApiError::BadRequest("explicit answer_value required".into()))?;
        field
            .validate_value(answer)
            .map_err(|e| ApiError::BadRequest(e.into()))?;
        nod_proto::value_signing::decision_signing_payload_v2(input, answer)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?
    } else {
        if answer.is_some() || provided.payload_version.is_some() {
            return Err(ApiError::BadRequest(
                "ordinary request does not accept value payload".into(),
            ));
        }
        nod_proto::decision_signing_payload(input)
    };

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
