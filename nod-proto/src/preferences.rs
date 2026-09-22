use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

/// Per-device alert controls. These never hide requests from the inbox or
/// change the immutable content a decision signature covers.
#[typeshare]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceNotificationPreferences {
    pub hide_content: bool,
    pub muted_channels: Vec<String>,
    #[typeshare(serialized_as = "Option<String>")]
    pub snoozed_until: Option<DateTime<Utc>>,
}

impl DeviceNotificationPreferences {
    pub fn allows_alert(&self, channel_id: &str, now: DateTime<Utc>) -> bool {
        !self
            .muted_channels
            .iter()
            .any(|channel| channel == channel_id)
            && self.snoozed_until.is_none_or(|until| until <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_controls_are_scoped_to_the_device_and_expire_at_the_boundary() {
        let now = Utc::now();
        let mut preferences = DeviceNotificationPreferences {
            hide_content: true,
            muted_channels: vec!["quiet".to_string()],
            snoozed_until: Some(now),
        };
        assert!(preferences.allows_alert("other", now));
        assert!(!preferences.allows_alert("quiet", now));
        preferences.snoozed_until = Some(now + chrono::Duration::seconds(1));
        assert!(!preferences.allows_alert("other", now));
    }
}
