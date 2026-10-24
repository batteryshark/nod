//! Additive value-only signing; historical encoders deliberately stay untouched.
use crate::{
    signing::{DecisionSigningInput, SigningError},
    AnswerField, AnswerValue, Request,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const VALUE_REQUEST_DIGEST_VERSION: &str = "nod-request-v3";
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

pub fn request_digest_v3(
    request: &Request,
    commitment: &str,
    field: &AnswerField,
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
    let hash = Sha256::digest(format!("{VALUE_REQUEST_DIGEST_VERSION}\n{canonical}\n").as_bytes());
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
    #[test]
    fn canonical_sorts_recursively_and_refuses_floats() {
        assert_eq!(
            canonical_value_json(&json!({"z":[{"y":"é/", "a":"\n"}], "a":null})).unwrap(),
            "{\"a\":null,\"z\":[{\"a\":\"\\n\",\"y\":\"é/\"}]}"
        );
        assert!(canonical_value_json(&json!({"n":1.5})).is_err());
    }
}
