use nod_client_core::models::{OptionKind, Request, RequestOption, RequestStatus};

use super::NotificationActivation;

const MAX_NOTIFICATION_OPTIONS: usize = 4;

pub(crate) fn value_unsupported(request: &Request) -> bool {
    nod_proto::is_value_request(
        request.answer_field.is_some(),
        request
            .signing
            .as_ref()
            .map(|signing| signing.version.as_str()),
    ) || request
        .options
        .iter()
        .any(|option| option.id == "__nod_value_open")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DesktopNotificationOption {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) activation: NotificationActivation,
}

pub(super) fn desktop_notification_options(
    server_id: &str,
    request: &Request,
) -> Vec<DesktopNotificationOption> {
    if request.status != RequestStatus::Pending {
        return Vec::new();
    }
    if value_unsupported(request)
        || request.notification.redact
        || request.options.len() > MAX_NOTIFICATION_OPTIONS
    {
        return vec![DesktopNotificationOption {
            id: "open".into(),
            label: "Open Nod".into(),
            activation: NotificationActivation::Open {
                server_id: server_id.to_string(),
                request_id: Some(request.id.clone()),
            },
        }];
    }
    if request.options.is_empty() {
        return vec![DesktopNotificationOption {
            id: "dismiss".to_string(),
            label: "Dismiss".to_string(),
            activation: NotificationActivation::Submit {
                server_id: server_id.to_string(),
                request_id: request.id.clone(),
                option_id: "dismiss".to_string(),
            },
        }];
    }

    request
        .options
        .iter()
        .map(|option| option_for_request(server_id, request, option))
        .collect()
}

fn option_for_request(
    server_id: &str,
    request: &Request,
    option: &RequestOption,
) -> DesktopNotificationOption {
    // OS notification buttons cannot collect free-form text, so text options open the request detail view.
    let activation = if option.foreground
        || option.kind == OptionKind::Open
        || option.requires_text
        || requires_text(&option.kind)
    {
        NotificationActivation::Open {
            server_id: server_id.to_string(),
            request_id: Some(request.id.clone()),
        }
    } else {
        NotificationActivation::Submit {
            server_id: server_id.to_string(),
            request_id: request.id.clone(),
            option_id: option.id.clone(),
        }
    };

    DesktopNotificationOption {
        id: option.id.clone(),
        label: option.label.clone(),
        activation,
    }
}

fn requires_text(kind: &OptionKind) -> bool {
    matches!(
        kind,
        OptionKind::ApproveWithText | OptionKind::RejectWithText
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(version: &str, with_value: bool) -> Request {
        let mut card: serde_json::Value = serde_json::from_str(
            r#"{"id":"r","request_id":"r","channel_id":"c","title":"t","summary":"","body_markdown":"","notification":{},"status":"pending","created_at":"2026-05-28T12:00:00Z","updated_at":"2026-05-28T12:00:00Z","options":[{"id":"approve","label":"Approve","kind":"approve"}]}"#,
        )
        .unwrap();
        card["signing"] = serde_json::json!({
            "version": version,
            "recipients_commitment": "a".repeat(64),
            "request_digest": "b".repeat(64)
        });
        if with_value {
            card["answer_field"] = serde_json::json!({
                "kind": "number",
                "label": "Count",
                "default": {"kind": "number", "decimal": "4"}
            });
        }
        serde_json::from_value(card).unwrap()
    }

    #[test]
    fn current_and_legacy_value_tags_open_in_app_but_legacy_ordinary_does_not() {
        for version in [
            nod_proto::value_signing::VALUE_REQUEST_DIGEST_VERSION,
            nod_proto::value_signing::LEGACY_VALUE_REQUEST_DIGEST_VERSION,
        ] {
            assert!(value_unsupported(&request(version, true)), "{version}");
        }
        assert!(!value_unsupported(&request(
            nod_proto::LEGACY_STRUCTURED_REQUEST_DIGEST_VERSION,
            false
        )));
    }
}
