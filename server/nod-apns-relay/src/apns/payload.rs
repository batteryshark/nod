use serde::Serialize;

use crate::relay::RelayNotification;

#[derive(Serialize)]
pub(crate) struct ApnsPayload<'a> {
    aps: ApsPayload<'a>,
    // Keep relay metadata outside `aps`; APNs reserves `aps` for delivery and
    // display controls.
    nod: ApnsMetadata<'a>,
}

#[derive(Serialize)]
struct ApsPayload<'a> {
    alert: ApnsAlert<'a>,
    #[serde(rename = "thread-id")]
    thread_id: &'a str,
    category: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<&'a str>,
}

#[derive(Serialize)]
struct ApnsAlert<'a> {
    title: &'a str,
    body: &'a str,
}

#[derive(Serialize)]
struct ApnsMetadata<'a> {
    request_id: &'a str,
    channel_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
}

pub(crate) fn apns_payload(notification: &RelayNotification) -> ApnsPayload<'_> {
    ApnsPayload {
        aps: ApsPayload {
            alert: ApnsAlert {
                title: &notification.notification.title,
                body: &notification.notification.body,
            },
            thread_id: &notification.notification.thread_id,
            category: &notification.notification.category,
            sound: apns_sound(&notification.notification.sound),
        },
        nod: ApnsMetadata {
            request_id: &notification.metadata.request_id,
            channel_id: &notification.metadata.channel_id,
            device_id: notification.metadata.device_id.as_deref(),
        },
    }
}

fn apns_sound(sound: &str) -> Option<&str> {
    match Some(sound.trim()).filter(|value| !value.is_empty()) {
        // A muted alert is represented by omitting `aps.sound`; the payload
        // still carries `aps.alert`.
        Some("none") | Some("silent") => None,
        Some(sound) => Some(sound),
        None => Some("default"),
    }
}

#[cfg(test)]
mod tests {
    use crate::relay::{
        RelayNotification, RelayNotificationContent, RelayNotificationMetadata, RelayTarget,
        TargetPlatform,
    };

    use super::*;

    #[test]
    fn apns_payload_maps_relay_notification() {
        let notification = valid_notification();

        let payload = serde_json::to_value(apns_payload(&notification)).unwrap();

        assert_eq!(payload["aps"]["alert"]["title"], "Deploy");
        assert_eq!(
            payload["aps"]["alert"]["body"],
            "Production deploy is waiting"
        );
        assert_eq!(payload["aps"]["thread-id"], "default");
        assert_eq!(payload["aps"]["category"], "NOD_APPROVAL");
        assert_eq!(payload["aps"]["sound"], "nod_ping.wav");
        assert_eq!(payload["nod"]["request_id"], "request-1");
        assert_eq!(payload["nod"]["channel_id"], "default");
        assert!(payload.get("request_id").is_none());
        assert!(payload.get("channel_id").is_none());
    }

    #[test]
    fn apns_payload_omits_silent_sound() {
        let mut notification = valid_notification();
        notification.notification.sound = "silent".to_string();

        let payload = serde_json::to_value(apns_payload(&notification)).unwrap();

        assert!(payload["aps"].get("sound").is_none());
    }

    #[test]
    fn apns_payload_defaults_blank_sound() {
        let mut notification = valid_notification();
        notification.notification.sound = " ".to_string();

        let payload = serde_json::to_value(apns_payload(&notification)).unwrap();

        assert_eq!(payload["aps"]["sound"], "default");
    }

    fn valid_notification() -> RelayNotification {
        RelayNotification {
            target: RelayTarget {
                platform: TargetPlatform::Ios,
                native_app_id: "com.example.NodTests".to_string(),
                token: "device-token".to_string(),
            },
            notification: RelayNotificationContent {
                title: "Deploy".to_string(),
                body: "Production deploy is waiting".to_string(),
                sound: "nod_ping.wav".to_string(),
                thread_id: "default".to_string(),
                category: "NOD_APPROVAL".to_string(),
            },
            metadata: RelayNotificationMetadata {
                request_id: "request-1".to_string(),
                channel_id: "default".to_string(),
                device_id: None,
            },
        }
    }
}
