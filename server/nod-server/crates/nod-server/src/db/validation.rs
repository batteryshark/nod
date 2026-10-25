use chrono::{DateTime, Utc};
use url::Url;

use crate::{
    error::ApiError,
    models::{CreateDecisionRequest, DecisionRequest, RequestOption},
};

pub(super) fn validate_request(req: &CreateDecisionRequest) -> Result<(), ApiError> {
    if let Some(field) = &req.answer_field {
        field
            .validate()
            .map_err(|e| ApiError::BadRequest(e.into()))?;
        if req
            .options
            .iter()
            .any(|o| o.id == "__nod_value_open" || o.kind == crate::models::OptionKind::Open)
            || !req.options.iter().any(|o| {
                !o.destructive
                    && !o.requires_text
                    && !matches!(
                        o.kind,
                        crate::models::OptionKind::ApproveWithText
                            | crate::models::OptionKind::RejectWithText
                            | crate::models::OptionKind::Open
                    )
            })
        {
            return Err(ApiError::BadRequest(
                "value requests require an ordinary one-tap outcome and no Open outcome".into(),
            ));
        }
    }
    if req.title.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "request title is required".to_string(),
        ));
    }
    if req.title.chars().count() > 200 {
        return Err(ApiError::BadRequest(
            "request title is too long".to_string(),
        ));
    }
    validate_notification_text(req.notification.title.as_deref(), "notification title", 200)?;
    validate_notification_text(req.notification.body.as_deref(), "notification body", 500)?;
    if let Some(callback_url) = req.callback_url.as_deref() {
        validate_http_url(callback_url, "callback_url")?;
    }
    // image_url and dedupe_key enter the v1 digest as raw lines, so a control
    // character in either could shift field boundaries. Links are opened by
    // clients and must be http(s) like everything else that leaves the card.
    if let Some(image_url) = req.image_url.as_deref() {
        reject_control_characters(image_url, "image_url")?;
        validate_http_url(image_url, "image_url")?;
    }
    if let Some(dedupe_key) = req.dedupe_key.as_deref() {
        reject_control_characters(dedupe_key, "dedupe_key")?;
    }
    for link in &req.links {
        validate_http_url(&link.url, "links[].url")?;
    }
    validate_id(&req.channel_id, "channel id")
}

fn reject_control_characters(value: &str, label: &str) -> Result<(), ApiError> {
    if value.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "{label} must not contain control characters"
        )));
    }
    Ok(())
}

fn validate_http_url(value: &str, label: &str) -> Result<(), ApiError> {
    let url = Url::parse(value)
        .map_err(|_| ApiError::BadRequest(format!("{label} must be an absolute URL")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ApiError::BadRequest(format!(
            "{label} must use http or https"
        )));
    }
    Ok(())
}

fn validate_notification_text(
    value: Option<&str>,
    label: &str,
    max_chars: usize,
) -> Result<(), ApiError> {
    if value
        .map(|value| value.trim().chars().count() > max_chars)
        .unwrap_or(false)
    {
        return Err(ApiError::BadRequest(format!("{label} is too long")));
    }
    Ok(())
}

pub(super) fn validate_id(value: &str, label: &str) -> Result<(), ApiError> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::BadRequest(format!(
            "{label} must be 1-80 ASCII slug characters"
        )));
    }
    Ok(())
}

pub(super) fn normalize_notification_sound(sound: &str) -> Result<String, ApiError> {
    let sound = sound.trim();
    if sound.is_empty() {
        return Ok("default".to_string());
    }
    if matches!(sound, "default" | "none" | "silent") {
        return Ok(sound.to_string());
    }
    if sound.len() > 80
        || !sound
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::BadRequest(
            "notification_sound must be default, none, silent, or a bundled .caf/.aiff/.wav filename"
                .to_string(),
        ));
    }
    let lowercase = sound.to_ascii_lowercase();
    if !(lowercase.ends_with(".caf") || lowercase.ends_with(".aiff") || lowercase.ends_with(".wav"))
    {
        return Err(ApiError::BadRequest(
            "custom notification sound files must use .caf, .aiff, or .wav".to_string(),
        ));
    }
    Ok(sound.to_string())
}

pub(super) fn normalize_options(options: Vec<RequestOption>) -> Vec<RequestOption> {
    options
        .into_iter()
        .map(|mut option| {
            if matches!(
                option.kind,
                crate::models::OptionKind::ApproveWithText
                    | crate::models::OptionKind::RejectWithText
            ) {
                option.requires_text = true;
            }
            option
        })
        .collect()
}

pub(super) fn implicit_dismiss_option(
    request: &DecisionRequest,
    option_id: &str,
) -> Option<RequestOption> {
    // Optionless notifications still need a completion action so clients can clear them from pending lists.
    if request.answer_field.is_some() || option_id != "dismiss" || !request.options.is_empty() {
        return None;
    }
    Some(RequestOption {
        id: "dismiss".to_string(),
        label: "Dismiss".to_string(),
        kind: crate::models::OptionKind::Dismiss,
        style: "default".to_string(),
        requires_text: false,
        text_placeholder: None,
        destructive: false,
        foreground: false,
    })
}

pub(super) fn parse_time(value: String) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(&value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| ApiError::Internal(format!("invalid timestamp in database: {err}")))
}

pub(super) fn parse_optional_time(
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    value.map(parse_time).transpose()
}
