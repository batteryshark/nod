use serde::Serialize;

use crate::models::{Decision, DecisionRequest, DecisionResolution, RequestStatus, UserDecision};

/// Decision-focused projection of a request (status, recorded decisions, and the
/// digest), used by the decision/callback responses. The full request wire shape
/// is `nod_proto::Request`, built via `DecisionRequest::to_wire`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestDecisionView {
    pub request_id: String,
    pub status: RequestStatus,
    pub decision: Option<Decision>,
    pub decisions: Vec<UserDecision>,
    pub decision_resolution: DecisionResolution,
    pub recipients: Vec<String>,
    pub pending_recipients: Vec<String>,
    pub request_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
}

impl RequestDecisionView {
    pub(crate) fn from_request(request: &DecisionRequest) -> Self {
        Self {
            request_id: request.id.clone(),
            status: request.status.clone(),
            decision: request.decision.clone(),
            decisions: request.user_decisions.clone(),
            decision_resolution: request.decision_resolution.clone(),
            recipients: request.recipients.clone(),
            pending_recipients: pending_recipients(request),
            request_digest: request.to_wire().request_digest,
            timed_out: None,
        }
    }

    pub(crate) fn mark_timed_out(&mut self) {
        self.timed_out = Some(true);
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CallbackPayload {
    pub request_id: String,
    pub channel_id: String,
    pub status: RequestStatus,
    pub decision: Option<Decision>,
    pub decisions: Vec<UserDecision>,
    pub decision_resolution: DecisionResolution,
}

impl CallbackPayload {
    pub(crate) fn from_request(request: &DecisionRequest) -> Self {
        let decision_view = RequestDecisionView::from_request(request);
        Self {
            request_id: decision_view.request_id,
            channel_id: request.channel_id.clone(),
            status: decision_view.status,
            decision: decision_view.decision,
            decisions: decision_view.decisions,
            decision_resolution: decision_view.decision_resolution,
        }
    }
}

fn pending_recipients(request: &DecisionRequest) -> Vec<String> {
    if request.decision_resolution != DecisionResolution::PerUser
        || request.status != RequestStatus::Pending
    {
        return Vec::new();
    }
    request
        .recipients
        .iter()
        .filter(|user_id| {
            !request
                .user_decisions
                .iter()
                .any(|decision| &decision.user_id == *user_id)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::models::{Decision, DecisionResolution, OptionKind, RequestStatus};

    #[test]
    fn callback_payload_uses_the_decision_projection() {
        let now = Utc::now();
        let decision = Decision {
            request_id: "request-1".to_string(),
            option_id: "approve".to_string(),
            option_kind: OptionKind::Approve,
            option_label: "Approve".to_string(),
            text: Some("ship it".to_string()),
            actor_user_id: Some("owner".to_string()),
            actor_device_id: Some("device-1".to_string()),
            signature: None,
            resolved_at: now,
        };
        let request = DecisionRequest {
            id: "request-1".to_string(),
            channel_id: "deploys".to_string(),
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
            status: RequestStatus::Resolved,
            created_at: now,
            updated_at: now,
            resolved_at: Some(now),
            decision: Some(decision),
            user_decisions: Vec::new(),
            callback_url: None,
            options: Vec::new(),
            private_recipients: false,
            signing: None,
        };

        let callback = serde_json::to_value(CallbackPayload::from_request(&request)).unwrap();
        let projection = serde_json::to_value(RequestDecisionView::from_request(&request)).unwrap();

        assert_eq!(callback["decision"], projection["decision"]);
        assert_eq!(callback["decisions"], projection["decisions"]);
        assert_eq!(
            callback["decision_resolution"],
            projection["decision_resolution"]
        );
    }
}
