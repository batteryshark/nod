use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// APNs transport boundary used by the HTTP relay.
#[async_trait]
pub trait ApnsDelivery: Send + Sync {
    async fn send(&self, notification: &RelayNotification) -> anyhow::Result<()>;
}

pub type DynApnsDelivery = Arc<dyn ApnsDelivery>;

/// Validation rules that keep the relay pinned to one APNs bundle id.
#[derive(Debug, Clone)]
pub struct RelayPolicy {
    expected_bundle_id: String,
}

impl RelayPolicy {
    pub fn new(expected_bundle_id: impl Into<String>) -> Self {
        Self {
            expected_bundle_id: expected_bundle_id.into(),
        }
    }

    fn validate(&self, request: ApnsRelayRequest) -> Result<RelayNotification, ApiError> {
        let target = RelayTarget {
            platform: TargetPlatform::parse(&request.target.platform)?,
            native_app_id: required_text("target.native_app_id", &request.target.native_app_id)?,
            token: required_text("target.token", &request.target.token)?,
        };
        if target.native_app_id != self.expected_bundle_id {
            return Err(ApiError::BadRequest(format!(
                "target.native_app_id {:?} does not match configured APNs bundle id {:?}",
                target.native_app_id, self.expected_bundle_id
            )));
        }

        Ok(RelayNotification {
            target,
            notification: RelayNotificationContent {
                title: required_text("notification.title", &request.notification.title)?,
                body: required_text("notification.body", &request.notification.body)?,
                sound: request.notification.sound.trim().to_string(),
                thread_id: required_text(
                    "notification.thread_id",
                    &request.notification.thread_id,
                )?,
                category: required_text("notification.category", &request.notification.category)?,
            },
            metadata: RelayNotificationMetadata {
                request_id: required_text("metadata.request_id", &request.metadata.request_id)?,
                channel_id: required_text("metadata.channel_id", &request.metadata.channel_id)?,
                device_id: request.metadata.device_id,
            },
        })
    }

    /// Validate and sanitize a relay request for in-process (library) delivery.
    ///
    /// The HTTP handler uses [`Self::validate`] directly for typed 4xx
    /// responses; co-located callers (the server embedding this crate) get the
    /// same bundle-id pinning and field validation, with the error mapped to
    /// `anyhow`.
    pub fn sanitize(&self, request: ApnsRelayRequest) -> anyhow::Result<RelayNotification> {
        self.validate(request).map_err(|err| {
            crate::error::DeliveryFailure {
                message: err.to_string(),
                retryable: false,
                invalid_token: false,
            }
            .into()
        })
    }
}

#[derive(Clone)]
pub struct RelayState {
    delivery: DynApnsDelivery,
    policy: RelayPolicy,
}

impl RelayState {
    pub fn new(delivery: DynApnsDelivery, policy: RelayPolicy) -> Self {
        Self { delivery, policy }
    }
}

pub fn router(delivery: DynApnsDelivery, policy: RelayPolicy) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/notifications", post(relay_notification))
        .with_state(RelayState::new(delivery, policy))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "nod-apns-relay",
    })
}

async fn relay_notification(
    State(state): State<RelayState>,
    Json(request): Json<ApnsRelayRequest>,
) -> Result<Json<AckResponse>, ApiError> {
    let notification = state.policy.validate(request)?;
    state.delivery.send(&notification).await.map_err(|err| {
        tracing::warn!(error = %err, "APNs delivery failed");
        if let Some(failure) = err.downcast_ref::<crate::error::DeliveryFailure>() {
            return ApiError::Delivery(failure.clone());
        }
        ApiError::Upstream("APNs delivery failed".to_string())
    })?;
    Ok(Json(AckResponse { ok: true }))
}

/// HTTP request body accepted from `nod-server`.
///
/// The nested `deny_unknown_fields` attributes keep contract drift from silently
/// reaching APNs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApnsRelayRequest {
    pub target: NotificationTarget,
    pub notification: NotificationContent,
    pub metadata: NotificationMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationTarget {
    pub platform: String,
    pub native_app_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationContent {
    pub title: String,
    pub body: String,
    pub sound: String,
    pub thread_id: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationMetadata {
    pub request_id: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// Sanitized notification passed to APNs delivery after request validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayNotification {
    pub target: RelayTarget,
    pub notification: RelayNotificationContent,
    pub metadata: RelayNotificationMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayTarget {
    pub platform: TargetPlatform,
    pub native_app_id: String,
    pub token: String,
}

/// Apple platforms that share the configured APNs bundle contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Ios,
    WatchOs,
}

impl TargetPlatform {
    fn parse(value: &str) -> Result<Self, ApiError> {
        let value = required_text("target.platform", value)?;
        match value.as_str() {
            "ios" => Ok(Self::Ios),
            "watchos" => Ok(Self::WatchOs),
            other => Err(ApiError::BadRequest(format!(
                "target.platform must be ios or watchos, got {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayNotificationContent {
    pub title: String,
    pub body: String,
    pub sound: String,
    pub thread_id: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayNotificationMetadata {
    pub request_id: String,
    pub channel_id: String,
    pub device_id: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Serialize)]
struct AckResponse {
    ok: bool,
}

fn required_text(field: &str, value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        Err(ApiError::BadRequest(format!("{field} is required")))
    } else {
        Ok(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Method, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use super::*;

    const TEST_BUNDLE_ID: &str = "com.example.NodTests";

    #[derive(Clone, Default)]
    struct RecordingDelivery {
        requests: Arc<Mutex<Vec<RelayNotification>>>,
        fail: bool,
    }

    #[async_trait]
    impl ApnsDelivery for RecordingDelivery {
        async fn send(&self, notification: &RelayNotification) -> anyhow::Result<()> {
            self.requests.lock().await.push(notification.clone());
            if self.fail {
                anyhow::bail!("mock APNs failure");
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn relay_forwards_valid_request_to_apns() {
        let delivery = RecordingDelivery::default();
        let app = test_router(delivery.clone());

        let (status, value) = post_json(app, valid_request()).await;

        assert_eq!(status, StatusCode::OK, "{value}");
        let requests = delivery.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].target.token, "device-token");
    }

    #[tokio::test]
    async fn relay_rejects_blank_required_fields() {
        let delivery = RecordingDelivery::default();
        let app = test_router(delivery.clone());
        let mut request = valid_request();
        request["target"]["token"] = serde_json::json!("");

        let (status, value) = post_json(app, request).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert!(delivery.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn relay_rejects_unknown_request_fields() {
        let delivery = RecordingDelivery::default();
        let app = test_router(delivery.clone());
        let mut request = valid_request();
        request["provider"] = serde_json::json!("apple_apns");

        let (status, _) = post_json(app, request).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(delivery.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn relay_rejects_unsupported_platform() {
        let delivery = RecordingDelivery::default();
        let app = test_router(delivery.clone());
        let mut request = valid_request();
        request["target"]["platform"] = serde_json::json!("android");

        let (status, value) = post_json(app, request).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert!(delivery.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn relay_rejects_wrong_native_app_id() {
        let delivery = RecordingDelivery::default();
        let app = test_router(delivery.clone());
        let mut request = valid_request();
        request["target"]["native_app_id"] = serde_json::json!("com.example.Other");

        let (status, value) = post_json(app, request).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert!(delivery.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn relay_reports_apns_failure() {
        let delivery = RecordingDelivery {
            fail: true,
            ..Default::default()
        };
        let app = test_router(delivery.clone());

        let (status, value) = post_json(app, valid_request()).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY, "{value}");
        assert_eq!(delivery.requests.lock().await.len(), 1);
    }

    async fn post_json(app: Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/notifications")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    fn test_router(delivery: RecordingDelivery) -> Router {
        router(Arc::new(delivery), RelayPolicy::new(TEST_BUNDLE_ID))
    }

    fn valid_request() -> serde_json::Value {
        serde_json::json!({
            "target": {
                "platform": "ios",
                "native_app_id": TEST_BUNDLE_ID,
                "token": "device-token"
            },
            "notification": {
                "title": "Deploy",
                "body": "Production deploy is waiting",
                "sound": "default",
                "thread_id": "default",
                "category": "NOD_APPROVAL"
            },
            "metadata": {
                "request_id": "request-1",
                "channel_id": "default"
            }
        })
    }
}
