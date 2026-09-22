use std::time::{Duration, Instant};

use nod_client_core::{models::Request, NodClientMessage};

const FLASH_DURATION: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlertEffect {
    pub ring_bell: bool,
}

#[derive(Debug, Clone)]
pub struct AlertState {
    muted: bool,
    active_request_id: Option<String>,
    message: Option<String>,
    flash_until: Option<Instant>,
}

impl Default for AlertState {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertState {
    pub fn new() -> Self {
        Self {
            muted: false,
            active_request_id: None,
            message: None,
            flash_until: None,
        }
    }

    pub fn apply_runtime_message(
        &mut self,
        request: &NodClientMessage,
        now: Instant,
    ) -> AlertEffect {
        match request {
            NodClientMessage::NotificationCandidate { request, .. } => self.alert_for(request, now),
            NodClientMessage::NotificationRemoved { request_id, .. } => {
                self.remove_request(request_id);
                AlertEffect::default()
            }
            _ => AlertEffect::default(),
        }
    }

    pub fn tick(&mut self, now: Instant) {
        if self
            .flash_until
            .map(|flash_until| now >= flash_until)
            .unwrap_or(false)
        {
            self.flash_until = None;
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            self.flash_until = None;
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn flashing(&self) -> bool {
        self.flash_until.is_some()
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn alert_for(&mut self, request: &Request, now: Instant) -> AlertEffect {
        self.active_request_id = Some(request.id.clone());
        self.message = Some(format!(
            "New request: {}",
            nod_proto::notification_preview(request).title
        ));

        if self.muted {
            return AlertEffect::default();
        }

        self.flash_until = Some(now + FLASH_DURATION);
        AlertEffect { ring_bell: true }
    }

    fn remove_request(&mut self, request_id: &str) {
        if self.active_request_id.as_deref() == Some(request_id) {
            self.active_request_id = None;
            self.message = None;
            self.flash_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nod_client_core::NodClientMessage;

    use super::*;
    use crate::test_support::request;

    #[test]
    fn notification_candidate_rings_once_and_flashes() {
        let now = Instant::now();
        let mut alerts = AlertState::new();
        let effect = alerts.apply_runtime_message(
            &NodClientMessage::NotificationCandidate {
                server_id: "local".into(),
                request: Box::new(request("new", "default")),
            },
            now,
        );

        assert!(effect.ring_bell);
        assert!(alerts.flashing());
        assert_eq!(alerts.message(), Some("New request: new"));

        alerts.tick(now + Duration::from_secs(2));
        assert!(!alerts.flashing());
        assert_eq!(alerts.message(), Some("New request: new"));
    }

    #[test]
    fn redacted_candidate_keeps_private_title_out_of_terminal_alert() {
        let mut candidate = request("private-title", "default");
        candidate.notification.redact = true;
        let mut alerts = AlertState::new();
        alerts.apply_runtime_message(
            &NodClientMessage::NotificationCandidate {
                server_id: "local".into(),
                request: Box::new(candidate),
            },
            Instant::now(),
        );
        assert_eq!(alerts.message(), Some("New request: Nod"));
    }

    #[test]
    fn mute_suppresses_bell_and_flash() {
        let now = Instant::now();
        let mut alerts = AlertState::new();
        alerts.toggle_mute();

        let effect = alerts.apply_runtime_message(
            &NodClientMessage::NotificationCandidate {
                server_id: "local".into(),
                request: Box::new(request("quiet", "default")),
            },
            now,
        );

        assert!(!effect.ring_bell);
        assert!(!alerts.flashing());
        assert_eq!(alerts.message(), Some("New request: quiet"));
    }

    #[test]
    fn removal_clears_active_alert() {
        let now = Instant::now();
        let mut alerts = AlertState::new();
        alerts.apply_runtime_message(
            &NodClientMessage::NotificationCandidate {
                server_id: "local".into(),
                request: Box::new(request("done", "default")),
            },
            now,
        );

        alerts.apply_runtime_message(
            &NodClientMessage::NotificationRemoved {
                server_id: "local".into(),
                request_id: "done".to_string(),
            },
            now,
        );

        assert!(!alerts.flashing());
        assert_eq!(alerts.message(), None);
    }
}
