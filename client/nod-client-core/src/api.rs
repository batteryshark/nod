use std::time::Duration;

use anyhow::{anyhow, Result};
use reqwest::{Client, Method, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::models::{
    Channel, ChannelsResponse, CurrentUserResponse, DecisionSignature, DevicePlatform,
    DeviceSigningKey, EnrollDeviceResponse, Request, RequestResponse, RequestsResponse, UserDevice,
    UserDeviceResponse, UserDevicesResponse,
};

const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct NodApi {
    base_url: Url,
    token: Option<String>,
    client: Client,
}

impl NodApi {
    pub(crate) fn http_client() -> Result<Client> {
        Ok(Client::builder().timeout(API_REQUEST_TIMEOUT).build()?)
    }

    pub(crate) fn with_client(
        base_url: &str,
        token: Option<String>,
        client: Client,
    ) -> Result<Self> {
        let base_url = Url::parse(&normalize_base_url(base_url))?;
        if !matches!(base_url.scheme(), "http" | "https") || base_url.host_str().is_none() {
            return Err(anyhow!(
                "server address must be an absolute HTTP or HTTPS URL"
            ));
        }
        Ok(Self {
            base_url,
            token,
            client,
        })
    }

    pub fn websocket_url(&self) -> Result<Url> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("device token is missing"))?;
        let mut url = self.base_url.clone();
        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .map_err(|_| anyhow!("could not set websocket scheme"))?;
        url.set_path(&join_path(self.base_url.path(), "/api/v1/sync"));
        url.query_pairs_mut().clear().append_pair("token", token);
        Ok(url)
    }

    pub async fn enroll(&self, request: EnrollDeviceRequest<'_>) -> Result<EnrollDeviceResponse> {
        self.request(RequestSpec::anonymous_json(
            Method::POST,
            "/api/v1/enroll",
            &request,
        ))
        .await
        .map_err(Into::into)
    }

    pub async fn current_user(&self) -> Result<CurrentUserResponse> {
        self.request(RequestSpec::authenticated(Method::GET, "/api/v1/users/me"))
            .await
            .map_err(Into::into)
    }

    pub async fn devices(&self) -> Result<Vec<UserDevice>> {
        let response: UserDevicesResponse = self
            .request(RequestSpec::authenticated(
                Method::GET,
                "/api/v1/users/me/devices",
            ))
            .await?;
        Ok(response.devices)
    }

    pub async fn rename_device(&self, id: &str, name: &str) -> Result<UserDevice> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
        }
        let path = format!("/api/v1/users/me/devices/{id}");
        let response: UserDeviceResponse = self
            .request(RequestSpec::authenticated_json(
                Method::PUT,
                &path,
                &Body { name },
            ))
            .await?;
        Ok(response.device)
    }

    pub async fn revoke_device(&self, id: &str) -> Result<()> {
        let path = format!("/api/v1/users/me/devices/{id}");
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated(Method::DELETE, &path))
            .await?;
        Ok(())
    }

    pub async fn channels(&self) -> Result<Vec<Channel>> {
        let response: ChannelsResponse = self
            .request(RequestSpec::authenticated(Method::GET, "/api/v1/channels"))
            .await?;
        Ok(response.channels)
    }

    pub async fn requests(
        &self,
        channel_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Request>> {
        let path = requests_path(channel_id, limit);
        let response: RequestsResponse = self
            .request(RequestSpec::authenticated(Method::GET, &path))
            .await?;
        Ok(response.requests)
    }

    pub async fn get_request(&self, request_id: &str) -> Result<Request> {
        let mut path = Url::parse("https://nod.invalid/api/v1/requests/")?;
        path.path_segments_mut()
            .map_err(|_| anyhow!("invalid request path"))?
            .pop_if_empty()
            .push(request_id);
        let response: RequestResponse = self
            .request(RequestSpec::authenticated(Method::GET, path.path()))
            .await?;
        Ok(response.request)
    }

    pub async fn query_history(
        &self,
        params: &crate::QueryHistoryParams,
    ) -> Result<RequestsResponse> {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("include_cleared", "true");
            for (name, value) in [
                ("channel_id", params.channel_id.as_deref()),
                ("search", params.search.as_deref()),
                ("before", params.before.as_deref()),
            ] {
                if let Some(value) = value {
                    query.append_pair(name, value);
                }
            }
            query.append_pair(
                "limit",
                &params.limit.unwrap_or(100).clamp(1, 500).to_string(),
            );
            format!("/api/v1/requests?{}", query.finish())
        };
        self.request(RequestSpec::authenticated(Method::GET, &path))
            .await
            .map_err(Into::into)
    }

    pub async fn submit_option(&self, request: SubmitOptionRequest<'_>) -> Result<Request> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            signature: Option<&'a DecisionSignature>,
        }
        let path = format!(
            "/api/v1/requests/{}/options/{}",
            request.request_id, request.option_id
        );
        let response: RequestResponse = self
            .request(RequestSpec::authenticated_json(
                Method::POST,
                &path,
                &Body {
                    text: request.text,
                    signature: request.signature,
                },
            ))
            .await?;
        Ok(response.request)
    }

    pub async fn clear_channel(&self, channel_id: &str) -> Result<()> {
        let path = format!("/api/v1/devices/me/channels/{channel_id}/clear");
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated(Method::POST, &path))
            .await?;
        Ok(())
    }

    pub async fn set_subscription(&self, channel_id: &str, subscribed: bool) -> Result<()> {
        #[derive(Serialize)]
        struct Body {
            subscribed: bool,
        }
        let path = format!("/api/v1/devices/me/subscriptions/{channel_id}");
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated_json(
                Method::PUT,
                &path,
                &Body { subscribed },
            ))
            .await?;
        Ok(())
    }

    pub async fn set_notification_sound(&self, notification_sound: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            notification_sound: &'a str,
        }
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated_json(
                Method::PUT,
                "/api/v1/devices/me/preferences",
                &Body { notification_sound },
            ))
            .await?;
        Ok(())
    }

    pub async fn set_device_notification_preferences(
        &self,
        preferences: &crate::models::DeviceNotificationPreferences,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated_json(
                Method::PUT,
                "/api/v1/devices/me/notification-preferences",
                preferences,
            ))
            .await?;
        Ok(())
    }

    /// Update the device's push registration after enrollment (e.g. when APNs
    /// issues or rotates a token). Mirrors NodKit's `updatePushToken`.
    pub async fn update_push_token(
        &self,
        provider: &str,
        native_app_id: &str,
        token: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            provider: &'a str,
            native_app_id: &'a str,
            token: &'a str,
        }
        let _: serde_json::Value = self
            .request(RequestSpec::authenticated_json(
                Method::PUT,
                "/api/v1/devices/me/push-token",
                &Body {
                    provider,
                    native_app_id,
                    token,
                },
            ))
            .await?;
        Ok(())
    }

    async fn request<Body, Response>(
        &self,
        spec: RequestSpec<'_, Body>,
    ) -> Result<Response, ApiStatusError>
    where
        Body: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        let url = self.url(spec.path).map_err(ApiStatusError::other)?;
        let mut request = self.client.request(spec.method, url);
        if let Some(body) = spec.body {
            request = request.json(body);
        }
        if spec.auth == RequestAuth::DeviceToken {
            let token = self
                .token
                .as_deref()
                .ok_or_else(|| ApiStatusError::other(anyhow!("device token is missing")))?;
            request = request.bearer_auth(token);
        }
        let response = request
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(ApiStatusError::other)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(ApiStatusError::other)?;
        if !status.is_success() {
            return Err(ApiStatusError {
                status,
                body: String::from_utf8_lossy(&bytes).to_string(),
            });
        }
        if bytes.is_empty() {
            return serde_json::from_value(serde_json::Value::Object(Default::default()))
                .map_err(ApiStatusError::other);
        }
        serde_json::from_slice(&bytes).map_err(ApiStatusError::other)
    }

    fn url(&self, path: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        let (path_part, query_part) = path.split_once('?').unwrap_or((path, ""));
        url.set_path(&join_path(self.base_url.path(), path_part));
        url.set_query(if query_part.is_empty() {
            None
        } else {
            Some(query_part)
        });
        Ok(url)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrollDeviceRequest<'a> {
    pub code: &'a str,
    pub device_name: &'a str,
    pub platform: DevicePlatform,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_app_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_provider: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_token: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<&'a DeviceSigningKey>,
    // App Attest payload built by the host (Apple); nod-client-core forwards it
    // opaquely so the canonical attestation shape stays the server's contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation: Option<&'a serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
pub struct SubmitOptionRequest<'a> {
    pub request_id: &'a str,
    pub option_id: &'a str,
    pub text: Option<&'a str>,
    pub signature: Option<&'a DecisionSignature>,
}

#[derive(Debug, thiserror::Error)]
#[error("Nod request failed with {status}: {body}")]
pub struct ApiStatusError {
    pub status: StatusCode,
    pub body: String,
}

impl ApiStatusError {
    fn other(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: error.to_string(),
        }
    }
}

#[derive(Debug)]
struct RequestSpec<'a, Body: ?Sized> {
    method: Method,
    path: &'a str,
    body: Option<&'a Body>,
    auth: RequestAuth,
}

impl<'a> RequestSpec<'a, ()> {
    fn authenticated(method: Method, path: &'a str) -> Self {
        Self {
            method,
            path,
            body: None,
            auth: RequestAuth::DeviceToken,
        }
    }
}

impl<'a, Body: ?Sized> RequestSpec<'a, Body> {
    fn anonymous_json(method: Method, path: &'a str, body: &'a Body) -> Self {
        Self {
            method,
            path,
            body: Some(body),
            auth: RequestAuth::Anonymous,
        }
    }

    fn authenticated_json(method: Method, path: &'a str, body: &'a Body) -> Self {
        Self {
            method,
            path,
            body: Some(body),
            auth: RequestAuth::DeviceToken,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestAuth {
    Anonymous,
    DeviceToken,
}

pub fn normalize_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    if trimmed == "localhost"
        || trimmed.starts_with("localhost:")
        || trimmed.starts_with("127.")
        || trimmed.starts_with("192.168.")
    {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    }
}

pub fn profile_id_for(base_url: &str) -> String {
    let normalized = normalize_base_url(base_url);
    let canonical = Url::parse(&normalized)
        .map(|mut url| {
            url.set_fragment(None);
            url.to_string().trim_end_matches('/').to_string()
        })
        .unwrap_or(normalized);
    format!("server-{:x}", Sha256::digest(canonical.as_bytes()))
}

pub fn display_name_for(base_url: &str) -> String {
    let normalized = normalize_base_url(base_url);
    let Ok(url) = Url::parse(&normalized) else {
        return "Nod Server".to_string();
    };
    let mut name = url.host_str().unwrap_or("Nod Server").to_string();
    let path = url.path().trim_matches('/');
    if !path.is_empty() {
        name.push('/');
        name.push_str(path);
    }
    name
}

fn join_path(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    match (base.is_empty(), path.is_empty()) {
        (true, true) => "/".to_string(),
        (true, false) => format!("/{path}"),
        (false, true) => base.to_string(),
        (false, false) => format!("{base}/{path}"),
    }
}

fn requests_path(channel_id: Option<&str>, limit: Option<usize>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("include_cleared", "false");
    if let Some(channel_id) = channel_id {
        query.append_pair("channel_id", channel_id);
    }
    if let Some(limit) = limit {
        query.append_pair("limit", &limit.to_string());
    }
    format!("/api/v1/requests?{}", query.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NotificationDeliveryMode, SyncEnvelope};

    #[test]
    fn normalizes_local_and_remote_urls() {
        assert_eq!(
            normalize_base_url("localhost:8767"),
            "http://localhost:8767"
        );
        assert_eq!(
            normalize_base_url("nod.example.com/api/"),
            "https://nod.example.com/api"
        );
    }

    #[test]
    fn builds_profile_ids_from_urls() {
        assert_eq!(
            profile_id_for("HTTPS://NOD.EXAMPLE.COM:443/api/"),
            profile_id_for("https://nod.example.com/api")
        );
        assert_ne!(
            profile_id_for("https://nod.example.com/a-b"),
            profile_id_for("https://nod.example.com/a/b")
        );
        assert_ne!(
            profile_id_for("https://nod.example.com/API"),
            profile_id_for("https://nod.example.com/api")
        );
    }

    #[test]
    fn builds_requests_path_with_encoded_filters() {
        assert_eq!(
            requests_path(Some("ops/main channel"), Some(25)),
            "/api/v1/requests?include_cleared=false&channel_id=ops%2Fmain+channel&limit=25"
        );
    }

    #[test]
    fn enroll_request_serializes_device_signing_key() {
        let signing_key = DeviceSigningKey {
            key_id: "key-1".to_string(),
            algorithm: "p256_ecdsa_sha256".to_string(),
            public_key: "public-key".to_string(),
        };
        let request = EnrollDeviceRequest {
            code: "ABC123",
            device_name: "Laptop",
            platform: DevicePlatform::Linux,
            native_app_id: None,
            push_provider: None,
            push_token: None,
            signing_key: Some(&signing_key),
            attestation: None,
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "code": "ABC123",
                "device_name": "Laptop",
                "platform": "linux",
                "signing_key": {
                    "key_id": "key-1",
                    "algorithm": "p256_ecdsa_sha256",
                    "public_key": "public-key"
                }
            })
        );
    }

    #[test]
    fn enrollment_response_decodes_channels_field() {
        let response: EnrollDeviceResponse = serde_json::from_value(serde_json::json!({
            "device_id": "device-1",
            "user_id": "owner",
            "user_name": "Owner",
            "token": "device-token",
            "notification_delivery": { "mode": "websocket" },
            "channels": [channel_json("default")],
            "devices": []
        }))
        .unwrap();

        assert_eq!(response.channels[0].id, "default");
    }

    #[test]
    fn channels_response_decodes_channels_field() {
        let response: ChannelsResponse = serde_json::from_value(serde_json::json!({
            "channels": [channel_json("default")]
        }))
        .unwrap();

        assert_eq!(response.channels[0].id, "default");
    }

    #[test]
    fn request_decodes_server_per_user_decision_field() {
        let request: Request =
            serde_json::from_value(request_json("request-1", "resolved")).unwrap();

        assert_eq!(request.decisions[0].user_id, "owner");
        assert_eq!(request.decisions[0].decision.option_id, "approve");
    }

    #[test]
    fn sync_payload_decodes_request_field() {
        let envelope: SyncEnvelope = serde_json::from_value(serde_json::json!({
            "kind": "created",
            "at": "2026-06-01T00:00:01Z",
            "payload": {
                "request": request_json("request-1", "pending")
            }
        }))
        .unwrap();

        assert_eq!(envelope.payload.request.unwrap().id, "request-1");
    }

    #[test]
    fn sync_payload_decodes_channel_field() {
        let envelope: SyncEnvelope = serde_json::from_value(serde_json::json!({
            "kind": "channel_updated",
            "at": "2026-06-01T00:00:01Z",
            "payload": {
                "channel": channel_json("default")
            }
        }))
        .unwrap();

        assert_eq!(envelope.payload.channel.unwrap().id, "default");
    }

    #[test]
    fn sync_payload_accepts_server_hello_notification_delivery() {
        let envelope: SyncEnvelope = serde_json::from_value(serde_json::json!({
            "kind": "hello",
            "at": "2026-06-01T00:00:01Z",
            "payload": {
                "device_id": "device-1",
                "notification_delivery": { "mode": "websocket" }
            }
        }))
        .unwrap();

        assert_eq!(
            envelope.payload.notification_delivery.unwrap().mode,
            NotificationDeliveryMode::Websocket
        );
        assert_eq!(
            envelope.payload.extra["device_id"].as_str(),
            Some("device-1")
        );
    }

    fn channel_json(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": "Default",
            "emoji": "🔔",
            "subscribed": true,
            "created_at": "2026-06-01T00:00:00Z"
        })
    }

    fn request_json(id: &str, status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "request_id": id,
            "channel_id": "default",
            "recipients": ["owner"],
            "decision_resolution": "per_user",
            "title": "Approve deployment",
            "summary": "Deployment is waiting",
            "body_markdown": "",
            "fields": [],
            "links": [],
            "image_url": null,
            "notification": {
                "redact": true,
                "title": "Nod",
                "body": "Open Nod to review this request."
            },
            "dedupe_key": null,
            "expires_at": null,
            "status": status,
            "created_at": "2026-06-01T00:00:00Z",
            "updated_at": "2026-06-01T00:00:01Z",
            "resolved_at": "2026-06-01T00:00:02Z",
            "decision": decision_json(id),
            "decisions": [
                {
                    "user_id": "owner",
                    "decision": decision_json(id)
                }
            ],
            "callback_url": null,
            "options": [
                {
                    "id": "approve",
                    "label": "Approve",
                    "kind": "approve"
                }
            ],
            "request_digest": "digest-1"
        })
    }

    fn decision_json(request_id: &str) -> serde_json::Value {
        serde_json::json!({
            "request_id": request_id,
            "option_id": "approve",
            "option_kind": "approve",
            "option_label": "Approve",
            "text": null,
            "actor_user_id": "owner",
            "actor_device_id": "device-1",
            "signature": null,
            "resolved_at": "2026-06-01T00:00:02Z"
        })
    }
}
