//! Request-card value types: the structured primitives that make up a Nod
//! decision request.
//!
//! These are part of the wire contract AND feed the request digest
//! (`CardField`/`CardLink` are serialized into it via `serde_json`), so their
//! serde shape is load-bearing — change them only alongside a corresponding
//! protocol-freeze vector.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::decision::{Decision, UserDecision};

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardField {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub style: Option<String>,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardLink {
    pub label: String,
    pub url: String,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    Approve,
    ApproveWithText,
    Reject,
    RejectWithText,
    Dismiss,
    Open,
    Custom,
}

impl OptionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::ApproveWithText => "approve_with_text",
            Self::Reject => "reject",
            Self::RejectWithText => "reject_with_text",
            Self::Dismiss => "dismiss",
            Self::Open => "open",
            Self::Custom => "custom",
        }
    }
}

impl From<&str> for OptionKind {
    fn from(value: &str) -> Self {
        match value {
            "approve" => Self::Approve,
            "approve_with_text" => Self::ApproveWithText,
            "reject" => Self::Reject,
            "reject_with_text" => Self::RejectWithText,
            "dismiss" => Self::Dismiss,
            "open" => Self::Open,
            _ => Self::Custom,
        }
    }
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOption {
    pub id: String,
    pub label: String,
    pub kind: OptionKind,
    #[serde(default = "default_option_style")]
    pub style: String,
    #[serde(default)]
    pub requires_text: bool,
    #[serde(default)]
    pub text_placeholder: Option<String>,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub foreground: bool,
}

fn default_option_style() -> String {
    "default".to_string()
}

#[typeshare]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestNotification {
    #[serde(default)]
    pub redact: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Resolved,
    Expired,
    Cancelled,
}

impl From<&str> for RequestStatus {
    fn from(value: &str) -> Self {
        match value {
            "resolved" => Self::Resolved,
            "expired" => Self::Expired,
            "cancelled" | "canceled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionResolution {
    Shared,
    PerUser,
}

impl DecisionResolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::PerUser => "per_user",
        }
    }
}

impl From<&str> for DecisionResolution {
    fn from(value: &str) -> Self {
        match value {
            "per_user" => Self::PerUser,
            _ => Self::Shared,
        }
    }
}

// --- Wire request DTOs -------------------------------------------------------

/// A Nod decision request as it appears on the wire. The server projects its
/// internal model into this shape; clients deserialize it directly. Unknown
/// fields are ignored, so a newer server can add wire fields without breaking
/// already-shipped clients.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub request_id: String,
    pub channel_id: String,
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(default = "default_decision_resolution")]
    pub decision_resolution: DecisionResolution,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body_markdown: String,
    #[serde(default)]
    pub fields: Vec<CardField>,
    #[serde(default)]
    pub links: Vec<CardLink>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub notification: RequestNotification,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    #[typeshare(serialized_as = "Option<String>")]
    pub expires_at: Option<DateTime<Utc>>,
    pub status: RequestStatus,
    #[typeshare(serialized_as = "String")]
    pub created_at: DateTime<Utc>,
    #[typeshare(serialized_as = "String")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    #[typeshare(serialized_as = "Option<String>")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default)]
    pub decisions: Vec<UserDecision>,
    #[serde(default)]
    pub callback_url: Option<String>,
    pub options: Vec<RequestOption>,
    #[serde(default)]
    pub request_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<RequestSigningContext>,
}

/// Content verification for a recipient-private request projection. The opaque
/// commitment binds hidden recipients without revealing their identities.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSigningContext {
    pub version: String,
    pub recipients_commitment: String,
    pub request_digest: String,
}

/// The issuer-facing body for creating a request. Strict
/// (`deny_unknown_fields`) so issuer typos are rejected rather than silently
/// dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDecisionRequest {
    #[serde(default = "default_channel")]
    pub channel_id: String,
    #[serde(default)]
    pub recipients: Option<Vec<String>>,
    #[serde(default)]
    pub decision_resolution: Option<DecisionResolution>,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body_markdown: String,
    #[serde(default)]
    pub fields: Vec<CardField>,
    #[serde(default)]
    pub links: Vec<CardLink>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub notification: RequestNotification,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub options: Vec<RequestOption>,
    #[serde(default)]
    pub callback_url: Option<String>,
}

/// The server's response to a successful create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedDecisionRequest {
    pub request_id: String,
    pub deduped: bool,
    pub request: Request,
}

fn default_decision_resolution() -> DecisionResolution {
    DecisionResolution::Shared
}

fn default_channel() -> String {
    "default".to_string()
}
