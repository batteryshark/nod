pub(crate) use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::{body::Body, http::Request, Router};
pub(crate) use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use http_body_util::BodyExt;
use nod_server::{router, AppState, Config};
pub(crate) use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
};
pub(crate) use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tempfile::TempDir;
use tower::ServiceExt;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TestApp {
    pub(crate) router: Router,
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) data_dir: std::path::PathBuf,
    /// Signing keys of devices enrolled through the helpers, by device token.
    signers: Mutex<HashMap<String, TestSigner>>,
    _tmp: TempDir,
}

/// A device's enrollment identity plus the in-test software key it signs with.
#[derive(Clone)]
pub(crate) struct TestSigner {
    pub(crate) device_id: String,
    pub(crate) user_id: String,
    pub(crate) key_id: String,
    pub(crate) private_key: String,
    pub(crate) public_key: String,
}

impl TestApp {
    pub(crate) async fn new() -> Self {
        Self::new_with_config(|_| {}).await
    }

    pub(crate) async fn new_with_config(configure: impl FnOnce(&mut Config)) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("nod.sqlite");
        let mut config = Config::with_admin_token("admin-test-token");
        config.bind = "127.0.0.1:0".to_string();
        config.database_url = format!("sqlite://{}", db_path.display());
        config.data_dir = tmp.path().join("data");
        configure(&mut config);
        let database_url = config.database_url.clone();
        let data_dir = config.data_dir.clone();
        let state = AppState::new(config).await.unwrap();
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
        Self {
            router: router(state),
            pool,
            data_dir,
            signers: Mutex::new(HashMap::new()),
            _tmp: tmp,
        }
    }

    /// The signing identity recorded for a device token enrolled via the helpers.
    pub(crate) fn signer_for(&self, device_token: &str) -> TestSigner {
        self.signers
            .lock()
            .unwrap()
            .get(device_token)
            .cloned()
            .expect("device token was enrolled through TestApp::enroll_device")
    }

    /// Submit a signed decision the way a first-party client does: fetch the
    /// request as this device sees it, sign the canonical payload with the
    /// device key, and POST it. `extra` merges into the body (e.g. `text`).
    pub(crate) async fn decide(
        &self,
        device_token: &str,
        request_id: &str,
        option_id: &str,
        extra: Value,
    ) -> (StatusCode, Value) {
        let signer = self.signer_for(device_token);
        let (status, view) = self
            .request(
                Method::GET,
                &format!("/api/v1/requests/{request_id}"),
                Some(device_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        let request: nod_proto::Request =
            serde_json::from_value(view["request"].clone()).expect("device request view");
        let option_kind = request
            .options
            .iter()
            .find(|option| option.id == option_id)
            .map(|option| option.kind.clone())
            .unwrap_or(nod_proto::OptionKind::Dismiss);
        let request_digest = request
            .request_digest
            .clone()
            .or_else(|| request.signing.as_ref().map(|s| s.request_digest.clone()))
            .expect("request digest in device view");
        let text = extra
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let nonce = format!(
            "test-nonce-{}",
            NONCE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let signed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let payload = nod_proto::decision_signing_payload(nod_proto::DecisionSigningInput {
            request_id,
            request_digest: &request_digest,
            option_id,
            option_kind: &option_kind,
            user_id: &signer.user_id,
            device_id: &signer.device_id,
            key_id: &signer.key_id,
            nonce: &nonce,
            signed_at: &signed_at,
            text,
        });
        let signature = nod_proto::sign_payload(&signer.private_key, payload.as_bytes()).unwrap();
        let mut body = match extra {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        body.insert(
            "signature".to_string(),
            json!({
                "key_id": signer.key_id,
                "algorithm": "p256_ecdsa_sha256",
                "nonce": nonce,
                "signed_at": signed_at,
                "request_digest": request_digest,
                "signature": signature
            }),
        );
        self.request(
            Method::POST,
            &format!("/api/v1/requests/{request_id}/options/{option_id}"),
            Some(device_token),
            Some(Value::Object(body)),
        )
        .await
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, _headers, value) = self.request_raw(method, uri, token, None, body).await;
        (status, value)
    }

    pub(crate) async fn request_with_cookie(
        &self,
        method: Method,
        uri: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        self.request_raw(method, uri, None, Some(cookie), body)
            .await
    }

    pub(crate) async fn request_raw(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        let request = if let Some(body) = body {
            builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, headers, value)
    }

    pub(crate) async fn request_text(
        &self,
        method: Method,
        uri: &str,
    ) -> (StatusCode, HeaderMap, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        (status, headers, text)
    }

    pub(crate) async fn enroll_device(&self, name: &str, platform: &str) -> (String, String) {
        self.enroll_device_for_user(name, platform, None).await
    }

    pub(crate) async fn enroll_device_for_user(
        &self,
        name: &str,
        platform: &str,
        user_id: Option<&str>,
    ) -> (String, String) {
        let user_id = user_id.unwrap_or("owner");
        let enrollment_body = json!({
            "expires_in_seconds": 600
        });
        let (status, enrollment) = self
            .request(
                Method::POST,
                &format!("/api/v1/admin/users/{user_id}/enrollment-codes"),
                Some("admin-test-token"),
                Some(enrollment_body),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{enrollment}");
        let code = enrollment["code"].as_str().unwrap();
        let key = nod_proto::generate_signing_key();
        let key_id = format!("test-key-{}", NONCE_COUNTER.fetch_add(1, Ordering::Relaxed));
        let (status, enrolled) = self
            .request(
                Method::POST,
                "/api/v1/enroll",
                None,
                Some(json!({
                    "code": code,
                    "device_name": name,
                    "platform": platform,
                    "signing_key": {
                        "key_id": key_id,
                        "algorithm": "p256_ecdsa_sha256",
                        "public_key": key.public_key
                    }
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{enrolled}");
        assert_eq!(enrolled["user_id"], user_id);
        let device_id = enrolled["device_id"].as_str().unwrap().to_string();
        let token = enrolled["token"].as_str().unwrap().to_string();
        self.signers.lock().unwrap().insert(
            token.clone(),
            TestSigner {
                device_id: device_id.clone(),
                user_id: user_id.to_string(),
                key_id,
                private_key: key.private_key,
                public_key: key.public_key,
            },
        );
        (device_id, token)
    }

    pub(crate) async fn create_user(&self, id: &str, name: &str) {
        let (status, value) = self
            .request(
                Method::POST,
                "/api/v1/admin/users",
                Some("admin-test-token"),
                Some(json!({ "id": id, "name": name })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
    }

    pub(crate) async fn issuer_token(&self) -> String {
        self.issuer_token_with_scopes(["requests:write", "requests:read"])
            .await
    }

    pub(crate) async fn issuer_token_with_scopes<const N: usize>(
        &self,
        scopes: [&str; N],
    ) -> String {
        let (status, value) = self
            .request(
                Method::POST,
                "/api/v1/admin/issuer-tokens",
                Some("admin-test-token"),
                Some(json!({
                    "name": "tests",
                    "scopes": scopes.to_vec()
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["token"].as_str().unwrap().to_string()
    }
}

/// A fresh in-test software key in the `signing_key` enrollment shape.
pub(crate) fn test_signing_key_json() -> Value {
    let key = nod_proto::generate_signing_key();
    json!({
        "key_id": format!("test-key-{}", NONCE_COUNTER.fetch_add(1, Ordering::Relaxed)),
        "algorithm": "p256_ecdsa_sha256",
        "public_key": key.public_key
    })
}

pub(crate) struct DecisionPayload<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_digest: &'a str,
    pub(crate) option_id: &'a str,
    pub(crate) option_kind: &'a str,
    pub(crate) user_id: &'a str,
    pub(crate) device_id: &'a str,
    pub(crate) key_id: &'a str,
    pub(crate) nonce: &'a str,
    pub(crate) signed_at: &'a str,
    pub(crate) text: &'a str,
}

pub(crate) fn decision_payload(payload: DecisionPayload<'_>) -> String {
    [
        "nod-decision-v1".to_string(),
        format!("request_id:{}", payload.request_id),
        format!("request_digest:{}", payload.request_digest),
        format!("option_id:{}", payload.option_id),
        format!("option_kind:{}", payload.option_kind),
        format!("user_id:{}", payload.user_id),
        format!("device_id:{}", payload.device_id),
        format!("key_id:{}", payload.key_id),
        format!("nonce:{}", payload.nonce),
        format!("signed_at:{}", payload.signed_at),
        format!(
            "text_sha256:{}",
            hex::encode(Sha256::digest(payload.text.as_bytes()))
        ),
        String::new(),
    ]
    .join("\n")
}
