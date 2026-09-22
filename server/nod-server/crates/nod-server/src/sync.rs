use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::models::{Channel, DecisionRequest, SyncEnvelope};

pub type SyncSender = broadcast::Sender<SyncEnvelope>;

pub fn sender() -> SyncSender {
    let (tx, _rx) = broadcast::channel(512);
    tx
}

pub fn request(kind: &str, request: &DecisionRequest) -> SyncEnvelope {
    targeted_envelope(kind, request_payload(request), request.recipients.clone())
}

pub fn request_for_users(
    kind: &str,
    request: &DecisionRequest,
    target_user_ids: Vec<String>,
) -> SyncEnvelope {
    targeted_envelope(kind, request_payload(request), target_user_ids)
}

pub fn channel_update(kind: &str, channel: &Channel) -> SyncEnvelope {
    envelope(kind, ChannelUpdate { channel })
}

pub fn device_update<T: Serialize>(kind: &str, payload: T) -> SyncEnvelope {
    envelope(kind, payload)
}

pub fn envelope<T: Serialize>(kind: &str, payload: T) -> SyncEnvelope {
    SyncEnvelope {
        kind: kind.to_string(),
        at: Utc::now(),
        target_user_ids: None,
        payload: to_payload(payload),
    }
}

pub fn targeted_envelope<T: Serialize>(
    kind: &str,
    payload: T,
    target_user_ids: Vec<String>,
) -> SyncEnvelope {
    SyncEnvelope {
        kind: kind.to_string(),
        at: Utc::now(),
        target_user_ids: Some(target_user_ids),
        payload: to_payload(payload),
    }
}

fn request_payload(request: &DecisionRequest) -> RequestUpdate {
    RequestUpdate {
        request: request.to_wire(),
    }
}

fn to_payload<T: Serialize>(payload: T) -> Value {
    serde_json::to_value(payload).unwrap_or(Value::Null)
}

#[derive(Serialize)]
struct RequestUpdate {
    request: nod_proto::Request,
}

#[derive(Serialize)]
struct ChannelUpdate<'a> {
    channel: &'a Channel,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::models::{DecisionResolution, RequestStatus};

    #[test]
    fn request_sync_payload_embeds_the_wire_request() {
        let now = Utc::now();
        let request = DecisionRequest {
            id: "request-1".to_string(),
            channel_id: "default".to_string(),
            recipients: vec!["owner".to_string()],
            decision_resolution: DecisionResolution::Shared,
            title: "Deploy".to_string(),
            summary: "Deploy is waiting".to_string(),
            body_markdown: String::new(),
            fields: Vec::new(),
            links: Vec::new(),
            image_url: None,
            notification: Default::default(),
            dedupe_key: None,
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            resolved_at: None,
            decision: None,
            user_decisions: Vec::new(),
            callback_url: None,
            options: Vec::new(),
            private_recipients: false,
            signing: None,
        };

        let envelope = request_payload(&request);
        let wire = serde_json::to_value(request.to_wire()).unwrap();

        assert_eq!(serde_json::to_value(envelope).unwrap()["request"], wire);
    }
}
