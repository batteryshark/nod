use serde::{Deserialize, Serialize};

use crate::Request;

const PREVIEW_TITLE_LIMIT: usize = 200;
const PREVIEW_BODY_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPreview {
    pub title: String,
    pub body: String,
    pub allow_image: bool,
}

/// One preview policy for remote push and local OS notifications. Redaction
/// removes content fallbacks and attachments, while preserving issuer-provided safe overrides.
pub fn notification_preview(request: &Request) -> NotificationPreview {
    let hints = &request.notification;
    let title = hints
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let body = hints
        .body
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let title = title.unwrap_or(if hints.redact { "Nod" } else { &request.title });
    let body = body.unwrap_or_else(|| {
        if hints.redact {
            "Open Nod to review this request."
        } else if !request.summary.trim().is_empty() {
            &request.summary
        } else if !request.body_markdown.trim().is_empty() {
            &request.body_markdown
        } else {
            "Open Nod to review this request."
        }
    });
    NotificationPreview {
        title: title.trim().chars().take(PREVIEW_TITLE_LIMIT).collect(),
        body: body.trim().chars().take(PREVIEW_BODY_LIMIT).collect(),
        allow_image: !hints.redact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        serde_json::from_value(serde_json::json!({
            "id": "r", "request_id": "r", "channel_id": "c", "title": "Private title",
            "summary": "Private summary", "status": "pending", "options": [],
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn redaction_never_falls_back_to_request_content_or_images() {
        let mut request = request();
        request.notification.redact = true;
        assert_eq!(
            notification_preview(&request),
            NotificationPreview {
                title: "Nod".into(),
                body: "Open Nod to review this request.".into(),
                allow_image: false,
            }
        );
    }

    #[test]
    fn explicit_safe_preview_overrides_survive_redaction() {
        let mut request = request();
        request.notification.redact = true;
        request.notification.title = Some("Review needed".into());
        request.notification.body = Some("An action is waiting".into());
        assert_eq!(notification_preview(&request).body, "An action is waiting");
    }

    #[test]
    fn title_only_requests_have_a_nonempty_body() {
        let mut request = request();
        request.summary.clear();
        assert!(!notification_preview(&request).body.is_empty());
    }

    #[test]
    fn preview_limits_count_characters_without_breaking_utf8() {
        let mut request = request();
        request.summary = "🔔".repeat(600);
        assert_eq!(
            notification_preview(&request).body.chars().count(),
            PREVIEW_BODY_LIMIT
        );
    }
}
