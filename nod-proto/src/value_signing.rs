//! Value-only signing: the `nod-request-v2` / `nod-decision-v2` pair for cards
//! with an `answer_field`. Historical value encoders stay frozen for
//! verification only.
use crate::{
    signing::{DecisionSigningInput, SigningError},
    AnswerField, AnswerValue, Request,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Version tag of the current value request digest. Pairs with
/// [`VALUE_DECISION_PAYLOAD_VERSION`]: these two are the only value scheme the
/// server issues.
pub const VALUE_REQUEST_DIGEST_VERSION: &str = "nod-request-v2";
/// Frozen tag of the pre-collapse value encoder (`nod-request-v3`). Kept only
/// so already-issued value decisions verify.
pub const LEGACY_VALUE_REQUEST_DIGEST_VERSION: &str = "nod-request-v3";
pub const VALUE_DECISION_PAYLOAD_VERSION: &str = "nod-decision-v2";

/// Compact recursively key-sorted JSON, refusing floating point inputs.
pub fn canonical_value_json(value: &Value) -> Result<String, &'static str> {
    fn write(value: &Value, out: &mut String) -> Result<(), &'static str> {
        match value {
            Value::Object(object) => {
                out.push('{');
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                for (i, key) in keys.iter().enumerate() {
                    if !key.is_ascii() {
                        return Err("non-ASCII canonical member key");
                    }
                    if i != 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).map_err(|_| "invalid key")?);
                    out.push(':');
                    write(&object[*key], out)?;
                }
                out.push('}');
            }
            Value::Array(array) => {
                out.push('[');
                for (i, item) in array.iter().enumerate() {
                    if i != 0 {
                        out.push(',');
                    }
                    write(item, out)?;
                }
                out.push(']');
            }
            Value::Number(n) if !n.is_i64() && !n.is_u64() => {
                return Err("floating canonical number")
            }
            _ => {
                out.push_str(&serde_json::to_string(value).map_err(|_| "invalid canonical value")?)
            }
        }
        Ok(())
    }
    let mut output = String::new();
    write(value, &mut output)?;
    Ok(output)
}

/// Recompute the value request digest for a card with an `answer_field`.
/// Emits tag [`VALUE_REQUEST_DIGEST_VERSION`] (`nod-request-v2`).
pub fn value_request_digest(
    request: &Request,
    commitment: &str,
    field: &AnswerField,
) -> Result<String, SigningError> {
    value_request_digest_with_version(request, commitment, field, VALUE_REQUEST_DIGEST_VERSION)
}

/// Frozen value digest under its pre-collapse tag
/// ([`LEGACY_VALUE_REQUEST_DIGEST_VERSION`], `nod-request-v3`).
///
/// **Verification of stored signatures only, never issuance.**
pub fn value_request_digest_legacy_v3(
    request: &Request,
    commitment: &str,
    field: &AnswerField,
) -> Result<String, SigningError> {
    value_request_digest_with_version(
        request,
        commitment,
        field,
        LEGACY_VALUE_REQUEST_DIGEST_VERSION,
    )
}

/// Deprecated alias for [`value_request_digest`]. The function used to emit the
/// `nod-request-v3` tag; it now emits the collapsed `nod-request-v2` tag.
#[deprecated(note = "use value_request_digest (nod-request-v2)")]
pub fn request_digest_v3(
    request: &Request,
    commitment: &str,
    field: &AnswerField,
) -> Result<String, SigningError> {
    value_request_digest(request, commitment, field)
}

/// Recompute the value request digest for a received signing-context tag.
///
/// `nod-request-v2` selects the current value encoder; `nod-request-v3` selects
/// the pre-collapse value encoder. Any other tag returns `Ok(None)` so the
/// caller keeps its existing unsupported-version error.
pub fn value_request_digest_for_version(
    request: &Request,
    version: &str,
    commitment: &str,
    field: &AnswerField,
) -> Result<Option<String>, SigningError> {
    match version {
        VALUE_REQUEST_DIGEST_VERSION => Ok(Some(value_request_digest(request, commitment, field)?)),
        LEGACY_VALUE_REQUEST_DIGEST_VERSION => Ok(Some(value_request_digest_legacy_v3(
            request, commitment, field,
        )?)),
        _ => Ok(None),
    }
}

fn value_request_digest_with_version(
    request: &Request,
    commitment: &str,
    field: &AnswerField,
    version: &str,
) -> Result<String, SigningError> {
    if commitment.len() != 64 || !commitment.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SigningError::InvalidRecipientCommitment);
    }
    field.validate().map_err(SigningError::InvalidValue)?;
    let content = json!({
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
        "expires_at": request.expires_at.map(|v| v.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        "created_at": request.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "callback_url": request.callback_url,
        "options": request.options,
        "answer_field": field,
    });
    let canonical = canonical_value_json(&content).map_err(SigningError::InvalidValue)?;
    let hash = Sha256::digest(format!("{version}\n{canonical}\n").as_bytes());
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn decision_signing_payload_v2(
    input: DecisionSigningInput<'_>,
    answer: &AnswerValue,
) -> Result<String, SigningError> {
    let signed_at = chrono::DateTime::parse_from_rfc3339(input.signed_at)
        .map_err(|_| SigningError::InvalidValue("invalid signed_at"))?;
    if signed_at
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        != input.signed_at
    {
        return Err(SigningError::InvalidValue("noncanonical signed_at"));
    }
    let text = input.text.map(str::trim).filter(|v| !v.is_empty());
    if text.is_some_and(|v| v.len() > 16384) {
        return Err(SigningError::InvalidValue("notes exceed 16384 bytes"));
    }
    let content = json!({
        "request_id": input.request_id,
        "request_digest": input.request_digest,
        "option_id": input.option_id,
        "option_kind": input.option_kind.as_str(),
        "user_id": input.user_id,
        "device_id": input.device_id,
        "key_id": input.key_id,
        "nonce": input.nonce,
        "signed_at": input.signed_at,
        "text": text,
        "answer_value": answer,
    });
    Ok(format!(
        "{VALUE_DECISION_PAYLOAD_VERSION}\n{}\n",
        canonical_value_json(&content).map_err(SigningError::InvalidValue)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DecisionSigningInput, OptionKind};

    #[test]
    fn canonical_sorts_recursively_and_refuses_floats() {
        assert_eq!(
            canonical_value_json(&json!({"z":[{"y":"é/", "a":"\n"}], "a":null})).unwrap(),
            "{\"a\":null,\"z\":[{\"a\":\"\\n\",\"y\":\"é/\"}]}"
        );
        assert!(canonical_value_json(&json!({"n":1.5})).is_err());
    }

    /// The collapsed scheme is numbered to pair with the decision payload:
    /// value requests are `nod-request-v2` + `nod-decision-v2`.
    #[test]
    fn value_scheme_is_tagged_nod_request_v2() {
        assert_eq!(VALUE_REQUEST_DIGEST_VERSION, "nod-request-v2");
        assert_eq!(VALUE_DECISION_PAYLOAD_VERSION, "nod-decision-v2");
        assert_eq!(LEGACY_VALUE_REQUEST_DIGEST_VERSION, "nod-request-v3");

        let request = value_request();
        let field = request.answer_field.clone().unwrap();
        let commitment =
            crate::recipients_commitment(&request.recipients, "fixed-test-salt").unwrap();
        // Frozen from the encoder with the collapsed tag.
        assert_eq!(
            value_request_digest(&request, &commitment, &field).unwrap(),
            "cc2015e3ddc9bad68dd666f90940852fb593f65079611c87cf387d21d7766b5d"
        );
        // Frozen before the collapse; same content, `nod-request-v3` tag.
        assert_eq!(
            value_request_digest_legacy_v3(&request, &commitment, &field).unwrap(),
            "bce730388e8ba18e9da08c115c763788f9b586b86149b732394b81ebe85dbe57"
        );
    }

    #[test]
    fn value_version_dispatch_selects_only_known_value_encoders() {
        let request = value_request();
        let field = request.answer_field.clone().unwrap();
        let commitment =
            crate::recipients_commitment(&request.recipients, "fixed-test-salt").unwrap();

        assert_eq!(
            value_request_digest_for_version(
                &request,
                VALUE_REQUEST_DIGEST_VERSION,
                &commitment,
                &field
            )
            .unwrap(),
            Some(value_request_digest(&request, &commitment, &field).unwrap())
        );
        assert_eq!(
            value_request_digest_for_version(
                &request,
                LEGACY_VALUE_REQUEST_DIGEST_VERSION,
                &commitment,
                &field
            )
            .unwrap(),
            Some(value_request_digest_legacy_v3(&request, &commitment, &field).unwrap())
        );
        assert_eq!(
            value_request_digest_for_version(
                &request,
                crate::REQUEST_DIGEST_VERSION,
                &commitment,
                &field
            )
            .unwrap(),
            None
        );
        assert_eq!(
            value_request_digest_for_version(&request, "nod-request-v999", &commitment, &field)
                .unwrap(),
            None
        );
    }

    /// `nod-decision-v2` payload bytes do not change in the digest collapse.
    #[test]
    fn value_decision_payload_v2_matches_frozen_contract() {
        let answer = AnswerValue::Number {
            decimal: "12".to_string(),
        };
        let payload = decision_signing_payload_v2(
            DecisionSigningInput {
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
            },
            &answer,
        )
        .unwrap();
        assert_eq!(
            payload,
            concat!(
                "nod-decision-v2\n",
                "{\"answer_value\":{\"decimal\":\"12\",\"kind\":\"number\"},",
                "\"device_id\":\"device-1\",",
                "\"key_id\":\"device-key-id\",",
                "\"nonce\":\"unique-device-nonce\",",
                "\"option_id\":\"approve\",",
                "\"option_kind\":\"approve\",",
                "\"request_digest\":\"server-provided-request-digest\",",
                "\"request_id\":\"request-1\",",
                "\"signed_at\":\"2026-05-31T12:00:00.000Z\",",
                "\"text\":\"ship it\",",
                "\"user_id\":\"user-1\"}\n"
            )
        );
    }

    fn value_request() -> Request {
        serde_json::from_value(json!({
            "answer_field": {
                "kind": "number",
                "label": "Quantity",
                "default": {"kind": "number", "decimal": "4"}
            },
            "id": "request-1",
            "request_id": "request-1",
            "channel_id": "deployments",
            "recipients": ["owner", "platform"],
            "decision_resolution": "shared",
            "title": "Approve production deploy",
            "summary": "api-gateway v42 is ready",
            "body_markdown": "**Change:** roll forward api-gateway to v42.",
            "fields": [{"label": "Service", "value": "api-gateway"}],
            "links": [{"label": "Diff", "url": "https://example.com/diff/42"}],
            "image_url": "https://example.com/canary.png",
            "dedupe_key": "deploy:api-gateway:42",
            "expires_at": "2027-01-01T00:10:00.000Z",
            "status": "pending",
            "created_at": "2026-05-28T12:00:00.000Z",
            "updated_at": "2026-05-28T12:00:00.000Z",
            "callback_url": "https://agent.example.com/nod/callback",
            "options": [{
                "id": "approve",
                "label": "Approve",
                "kind": "approve",
                "style": "default"
            }]
        }))
        .unwrap()
    }
}
