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
use tempfile::TempDir;
use tower::ServiceExt;

pub(crate) struct TestApp {
    pub(crate) router: Router,
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) data_dir: std::path::PathBuf,
    _tmp: TempDir,
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
            _tmp: tmp,
        }
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
        let (status, enrolled) = self
            .request(
                Method::POST,
                "/api/v1/enroll",
                None,
                Some(json!({
                    "code": code,
                    "device_name": name,
                    "platform": platform
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{enrolled}");
        assert_eq!(enrolled["user_id"], user_id);
        (
            enrolled["device_id"].as_str().unwrap().to_string(),
            enrolled["token"].as_str().unwrap().to_string(),
        )
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
