#[cfg(any(target_os = "linux", target_os = "windows", test))]
mod options;
mod platform;
#[cfg(any(target_os = "windows", test))]
mod windows_toast;

use std::sync::Arc;

use chrono::Timelike;
use nod_client_core::models::Request;
use tokio::sync::{Mutex, RwLock};

use crate::preferences::PreferenceStore;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use tokio::sync::mpsc;

#[cfg(target_os = "windows")]
pub(crate) use self::platform::register_toast_app_id;
use self::platform::{remove_notification, show_notification};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "windows", test))]
pub(crate) enum NotificationActivation {
    Open {
        server_id: String,
        request_id: Option<String>,
    },
    Submit {
        server_id: String,
        request_id: String,
        option_id: String,
    },
}

#[derive(Debug, Clone)]
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) struct NotificationContext {
    pub server_id: String,
    pub sound: String,
}

#[derive(Clone)]
pub(crate) struct DesktopNotifier {
    preferences: Arc<Mutex<PreferenceStore>>,
    sound: Arc<RwLock<String>>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    activations: mpsc::Sender<NotificationActivation>,
}

impl DesktopNotifier {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn new(
        activations: mpsc::Sender<NotificationActivation>,
        preferences: Arc<Mutex<PreferenceStore>>,
    ) -> Self {
        Self {
            activations,
            preferences,
            sound: Arc::new(RwLock::new("default".into())),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn new(preferences: Arc<Mutex<PreferenceStore>>) -> Self {
        Self {
            preferences,
            sound: Arc::new(RwLock::new("default".into())),
        }
    }

    pub(crate) async fn set_sound(&self, sound: &str) {
        *self.sound.write().await = sound.to_string();
    }

    pub(crate) async fn show(&self, server_id: &str, request: &Request) -> anyhow::Result<()> {
        let preferences = self.preferences.lock().await.get();
        let now = chrono::Local::now();
        if preferences.notifications_paused(
            &format!("{server_id}:{}", request.channel_id),
            now.timestamp(),
            now.hour() as u8,
        ) {
            return Ok(());
        }
        let mut request = request.clone();
        if preferences.hide_notification_content {
            request.notification.redact = true;
            request.notification.title = None;
            request.notification.body = None;
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let context = NotificationContext {
            server_id: server_id.into(),
            sound: self.sound.read().await.clone(),
        };
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            return show_notification(&request, &context, self.activations.clone()).await;
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            show_notification(&request).await
        }
    }

    pub(crate) async fn remove(&self, server_id: &str, request_id: &str) -> anyhow::Result<()> {
        remove_notification(server_id, request_id).await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use nod_client_core::models::{DecisionResolution, OptionKind, RequestOption, RequestStatus};

    use super::*;
    use super::{options::desktop_notification_options, windows_toast::windows_toast_xml};

    fn request(options: Vec<RequestOption>) -> Request {
        Request {
            id: "request-1".to_string(),
            request_id: "request-1".to_string(),
            channel_id: "default".to_string(),
            recipients: Vec::new(),
            decision_resolution: DecisionResolution::Shared,
            title: "Approve deploy".to_string(),
            summary: "Production deploy".to_string(),
            body_markdown: String::new(),
            fields: Vec::new(),
            links: Vec::new(),
            image_url: None,
            notification: Default::default(),
            dedupe_key: None,
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap(),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: None,
            options,
            request_digest: Some("digest".to_string()),
            signing: None,
        }
    }

    fn option(id: &str, kind: OptionKind, requires_text: bool) -> RequestOption {
        RequestOption {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            style: "default".to_string(),
            requires_text,
            text_placeholder: None,
            destructive: false,
            foreground: false,
        }
    }

    #[test]
    fn default_option_dismisses_from_notification() {
        let request = request(Vec::new());
        let options = desktop_notification_options("server-1", &request);

        assert_eq!(
            options[0].activation,
            NotificationActivation::Submit {
                server_id: "server-1".into(),
                request_id: "request-1".to_string(),
                option_id: "dismiss".to_string()
            }
        );
    }

    #[test]
    fn simple_options_submit_from_notification() {
        let request = request(vec![option("approve", OptionKind::Approve, false)]);
        let options = desktop_notification_options("server-1", &request);

        assert_eq!(
            options[0].activation,
            NotificationActivation::Submit {
                server_id: "server-1".into(),
                request_id: "request-1".to_string(),
                option_id: "approve".to_string()
            }
        );
    }

    #[test]
    fn text_options_open_request_detail() {
        let request = request(vec![option(
            "approve_notes",
            OptionKind::ApproveWithText,
            true,
        )]);
        let options = desktop_notification_options("server-1", &request);

        assert_eq!(
            options[0].activation,
            NotificationActivation::Open {
                server_id: "server-1".into(),
                request_id: Some("request-1".to_string())
            }
        );
    }

    #[test]
    fn oversized_option_sets_open_the_complete_request() {
        let request = request(vec![
            option("one", OptionKind::Custom, false),
            option("two", OptionKind::Custom, false),
            option("three", OptionKind::Custom, false),
            option("four", OptionKind::Custom, false),
            option("five", OptionKind::Custom, false),
        ]);

        let options = desktop_notification_options("server-1", &request);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].label, "Open Nod");
        assert!(matches!(
            options[0].activation,
            NotificationActivation::Open { .. }
        ));
    }

    #[test]
    fn redacted_toasts_do_not_leak_custom_option_labels() {
        let mut request = request(vec![option(
            "secret_customer_action",
            OptionKind::Custom,
            false,
        )]);
        request.notification.redact = true;
        let xml = windows_toast_xml(&request, "default");
        assert!(!xml.contains("secret_customer_action"));
        assert!(xml.contains("Open Nod"));
        assert!(matches!(
            desktop_notification_options("server-1", &request)[0].activation,
            NotificationActivation::Open { .. }
        ));
    }

    #[test]
    fn windows_xml_escapes_text_and_contains_options() {
        let request = Request {
            title: "Deploy <prod>".to_string(),
            summary: "A&B".to_string(),
            ..request(vec![option("approve", OptionKind::Approve, false)])
        };
        let xml = windows_toast_xml(&request, "default");

        assert!(xml.contains("Deploy &lt;prod&gt;"));
        assert!(xml.contains("A&amp;B"));
        assert!(xml.contains("<actions><action content=\"approve\" arguments=\"action:approve\""));
        assert!(!xml.contains("<options>"));
    }
    #[test]
    fn windows_tag_is_bounded_and_scoped() {
        use super::windows_toast::notification_tag;
        assert_eq!(notification_tag("request-one").len(), 16);
        assert_ne!(
            notification_tag("request-one"),
            notification_tag("request-two")
        );
    }

    #[test]
    fn redacted_silent_toast_has_no_request_content_or_actions_when_resolved() {
        let mut request = request(Vec::new());
        request.status = RequestStatus::Resolved;
        request.notification.redact = true;
        let xml = windows_toast_xml(&request, "silent");
        assert!(!xml.contains("Production"));
        assert!(!xml.contains("Approve deploy"));
        assert!(!xml.contains("<action "));
        assert!(xml.contains("<audio silent=\"true\"/>"));
    }
}
