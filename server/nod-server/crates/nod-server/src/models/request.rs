use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use nod_proto::{
    CardField, CardLink, CreateDecisionRequest, Decision, DecisionResolution,
    DecisionSignatureRecord, OptionKind, RequestNotification, RequestOption, RequestStatus,
    SubmitDecisionRequest, UserDecision,
};
// Clients submit, and the server verifies, the same signature shape.
pub use nod_proto::DecisionSignature as SubmitDecisionSignature;

/// Internal server model for a decision request.
///
/// Projected onto the canonical wire type [`nod_proto::Request`] via
/// [`DecisionRequest::to_wire`]. Differs from the wire shape deliberately: it
/// carries `user_decisions` (the wire calls it `decisions`) and omits the
/// duplicated `request_id`; the wire `request_digest` is computed at
/// projection time for full snapshots and single-recipient views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub id: String,
    pub channel_id: String,
    pub recipients: Vec<String>,
    pub decision_resolution: DecisionResolution,
    pub title: String,
    pub summary: String,
    pub body_markdown: String,
    pub fields: Vec<CardField>,
    pub links: Vec<CardLink>,
    pub image_url: Option<String>,
    pub notification: RequestNotification,
    pub dedupe_key: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: RequestStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub decision: Option<Decision>,
    #[serde(default)]
    pub user_decisions: Vec<UserDecision>,
    pub callback_url: Option<String>,
    pub options: Vec<RequestOption>,
    /// Multi-recipient device projections omit the unsalted v1 digest: it
    /// otherwise permits guessing hidden recipient IDs offline.
    #[serde(skip)]
    pub private_recipients: bool,
    pub signing: Option<nod_proto::RequestSigningContext>,
}

/// Server-internal result of creating a request (carries the internal model).
#[derive(Debug, Clone, Serialize)]
pub struct CreatedDecisionRequest {
    pub request_id: String,
    pub deduped: bool,
    pub request: DecisionRequest,
}

impl DecisionRequest {
    /// Full snapshots retain the legacy digest. Private device views use
    /// the independently verifiable, salted v2 signing context instead.
    pub fn to_wire(&self) -> nod_proto::Request {
        let mut wire = nod_proto::Request::from(self);
        if !self.private_recipients {
            wire.request_digest = nod_proto::request_digest(&wire).ok();
        }
        wire
    }
}

impl From<&DecisionRequest> for nod_proto::Request {
    fn from(request: &DecisionRequest) -> Self {
        nod_proto::Request {
            id: request.id.clone(),
            request_id: request.id.clone(),
            channel_id: request.channel_id.clone(),
            recipients: request.recipients.clone(),
            decision_resolution: request.decision_resolution.clone(),
            title: request.title.clone(),
            summary: request.summary.clone(),
            body_markdown: request.body_markdown.clone(),
            fields: request.fields.clone(),
            links: request.links.clone(),
            image_url: request.image_url.clone(),
            notification: request.notification.clone(),
            dedupe_key: request.dedupe_key.clone(),
            expires_at: request.expires_at,
            status: request.status.clone(),
            created_at: request.created_at,
            updated_at: request.updated_at,
            resolved_at: request.resolved_at,
            decision: request.decision.clone(),
            decisions: request.user_decisions.clone(),
            callback_url: request.callback_url.clone(),
            options: request.options.clone(),
            request_digest: None,
            signing: request.signing.clone(),
        }
    }
}
