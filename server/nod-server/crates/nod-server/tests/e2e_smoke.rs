//! End-to-end smoke test over a real TCP listener.
//!
//! The rest of the suite drives the router in-process via `tower::oneshot`,
//! which never opens a socket — this is the only coverage of the sync
//! WebSocket, and it asserts the socket's per-user request projection agrees
//! with the HTTP views. The same flow doubles as a deployed-instance verifier
//! (see `scripts/nod-smoke`):
//!
//! ```text
//! NOD_SMOKE_URL=https://nod.example NOD_SMOKE_ADMIN_TOKEN=... \
//!     cargo test -p nod-server --test e2e_smoke -- --ignored
//! ```
//!
//! Every resource is created under a unique `smoke-` prefix and deleted on
//! success; a failed run leaves the residue behind for inspection.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use nod_proto::signing::{
    decision_signing_payload, generate_signing_key, sign_payload, DecisionSigningInput,
};
use nod_proto::OptionKind;
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const WS_DEADLINE: Duration = Duration::from_secs(10);
const HEALTH_DEADLINE: Duration = Duration::from_secs(10);

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[tokio::test]
async fn smoke_in_process_server() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = nod_server::Config::with_admin_token("smoke-admin-token");
    config.bind = "127.0.0.1:0".to_string();
    config.database_url = format!("sqlite://{}", tmp.path().join("nod.sqlite").display());
    config.data_dir = tmp.path().join("data");
    let state = nod_server::AppState::new(config).await.unwrap();
    let router = nod_server::router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    run_smoke(&format!("http://{addr}"), "smoke-admin-token").await;
}

#[tokio::test]
#[ignore = "deployed-instance smoke: set NOD_SMOKE_URL and NOD_SMOKE_ADMIN_TOKEN"]
async fn smoke_deployed_instance() {
    let base_url = std::env::var("NOD_SMOKE_URL").expect("NOD_SMOKE_URL is not set");
    let admin_token =
        std::env::var("NOD_SMOKE_ADMIN_TOKEN").expect("NOD_SMOKE_ADMIN_TOKEN is not set");
    run_smoke(base_url.trim_end_matches('/'), &admin_token).await;
}

async fn run_smoke(base_url: &str, admin_token: &str) {
    let http = Client::new();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let user_id = format!("smoke-user-{suffix}");
    let watcher_user_id = format!("smoke-watcher-{suffix}");
    let channel_id = format!("smoke-{suffix}");

    wait_for_health(&http, base_url).await;

    // Admin provisioning: user, channel, enrollment code, issuer token.
    api(
        &http,
        base_url,
        Method::POST,
        "/api/v1/admin/users",
        admin_token,
        Some(json!({ "id": user_id, "name": "Smoke Test User" })),
    )
    .await;
    // A second recipient pins shared-resolution fanout: when the first user
    // decides, the watcher's socket must still hear about it.
    api(
        &http,
        base_url,
        Method::POST,
        "/api/v1/admin/users",
        admin_token,
        Some(json!({ "id": watcher_user_id, "name": "Smoke Watcher" })),
    )
    .await;
    api(
        &http,
        base_url,
        Method::POST,
        "/api/v1/admin/channels",
        admin_token,
        Some(json!({ "id": channel_id, "name": "Smoke", "emoji": "🧪" })),
    )
    .await;
    let enrollment = api(
        &http,
        base_url,
        Method::POST,
        &format!("/api/v1/admin/users/{user_id}/enrollment-codes"),
        admin_token,
        Some(json!({ "expires_in_seconds": 600 })),
    )
    .await;
    let code = enrollment["code"].as_str().expect("enrollment code");
    let issuer = api(
        &http,
        base_url,
        Method::POST,
        "/api/v1/admin/issuer-tokens",
        admin_token,
        Some(json!({ "name": format!("smoke-{suffix}"), "scopes": ["requests:write", "requests:read"] })),
    )
    .await;
    let issuer_token = issuer["token"].as_str().expect("issuer token").to_string();

    // Enroll a device carrying a fresh P-256 signing key.
    let key = generate_signing_key();
    let key_id = format!("smoke-key-{suffix}");
    let enrolled = http
        .post(format!("{base_url}/api/v1/enroll"))
        .json(&json!({
            "code": code,
            "device_name": "Smoke Device",
            "platform": "linux",
            "signing_key": {
                "key_id": key_id,
                "algorithm": "p256_ecdsa_sha256",
                "public_key": key.public_key
            }
        }))
        .send()
        .await
        .expect("enroll request");
    assert_eq!(enrolled.status(), StatusCode::OK, "enroll failed");
    let enrolled: Value = enrolled.json().await.expect("enroll body");
    let device_id = enrolled["device_id"]
        .as_str()
        .expect("device id")
        .to_string();
    let device_token = enrolled["token"]
        .as_str()
        .expect("device token")
        .to_string();

    // Subscribe the device's user to the smoke channel.
    api(
        &http,
        base_url,
        Method::PUT,
        &format!("/api/v1/devices/me/subscriptions/{channel_id}"),
        &device_token,
        Some(json!({ "subscribed": true })),
    )
    .await;

    // Enroll and subscribe the watcher's device (no signing key — it only
    // observes the sync stream).
    let watcher_enrollment = api(
        &http,
        base_url,
        Method::POST,
        &format!("/api/v1/admin/users/{watcher_user_id}/enrollment-codes"),
        admin_token,
        Some(json!({ "expires_in_seconds": 600 })),
    )
    .await;
    let watcher_code = watcher_enrollment["code"].as_str().expect("watcher code");
    let watcher_enrolled = http
        .post(format!("{base_url}/api/v1/enroll"))
        .json(&json!({
            "code": watcher_code,
            "device_name": "Smoke Watcher Device",
            "platform": "linux"
        }))
        .send()
        .await
        .expect("watcher enroll request");
    assert_eq!(
        watcher_enrolled.status(),
        StatusCode::OK,
        "watcher enroll failed"
    );
    let watcher_enrolled: Value = watcher_enrolled.json().await.expect("watcher enroll body");
    let watcher_device_id = watcher_enrolled["device_id"]
        .as_str()
        .expect("watcher device id")
        .to_string();
    let watcher_token = watcher_enrolled["token"]
        .as_str()
        .expect("watcher device token")
        .to_string();
    api(
        &http,
        base_url,
        Method::PUT,
        &format!("/api/v1/devices/me/subscriptions/{channel_id}"),
        &watcher_token,
        Some(json!({ "subscribed": true })),
    )
    .await;

    // Sync sockets: the server greets each device with `hello`.
    let ws_base = if let Some(rest) = base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        format!(
            "ws://{}",
            base_url.strip_prefix("http://").unwrap_or(base_url)
        )
    };
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("{ws_base}/api/v1/sync?token={device_token}"))
            .await
            .expect("sync websocket connect");
    let hello = next_ws_envelope(&mut ws, |envelope| envelope["kind"] == "hello").await;
    assert_eq!(hello["payload"]["device_id"], device_id.as_str(), "{hello}");
    let (mut watcher_ws, _) =
        tokio_tungstenite::connect_async(format!("{ws_base}/api/v1/sync?token={watcher_token}"))
            .await
            .expect("watcher sync websocket connect");
    let watcher_hello =
        next_ws_envelope(&mut watcher_ws, |envelope| envelope["kind"] == "hello").await;
    assert_eq!(
        watcher_hello["payload"]["device_id"],
        watcher_device_id.as_str(),
        "{watcher_hello}"
    );

    // Issuer creates a request in the smoke channel.
    let created = api(
        &http,
        base_url,
        Method::POST,
        "/api/v1/requests",
        &issuer_token,
        Some(json!({
            "channel_id": channel_id,
            "title": "Smoke: approve this request",
            "summary": "End-to-end smoke check",
            "options": [
                { "id": "approve", "label": "Approve", "kind": "approve" },
                { "id": "reject", "label": "Reject", "kind": "reject" }
            ]
        })),
    )
    .await;
    let request_id = created["request_id"]
        .as_str()
        .expect("request id")
        .to_string();
    let request_digest = created["request"]["request_digest"]
        .as_str()
        .expect("request digest")
        .to_string();
    assert_eq!(
        created["request"]["recipients"],
        json!([user_id, watcher_user_id]),
        "issuer create response should carry the canonical full-recipient snapshot"
    );

    // The `created` envelope reaches the device, and the socket's per-user
    // projection agrees with the HTTP device view of the same request.
    let ws_created = next_ws_envelope(&mut ws, |envelope| {
        envelope["kind"] == "created" && envelope["payload"]["request"]["id"] == request_id.as_str()
    })
    .await;
    let listed = api(
        &http,
        base_url,
        Method::GET,
        "/api/v1/requests",
        &device_token,
        None,
    )
    .await;
    let listed_request = listed["requests"]
        .as_array()
        .expect("requests array")
        .iter()
        .find(|request| request["id"] == request_id.as_str())
        .unwrap_or_else(|| panic!("request {request_id} missing from device list: {listed}"))
        .clone();
    let fetched_before_decision = api(
        &http,
        base_url,
        Method::GET,
        &format!("/api/v1/requests/{request_id}"),
        &device_token,
        None,
    )
    .await;
    assert_eq!(
        ws_created["payload"]["request"], listed_request,
        "sync projection diverges from the HTTP device view"
    );
    assert_eq!(
        fetched_before_decision["request"], listed_request,
        "HTTP get projection diverges from the HTTP list view"
    );
    let watcher_created = next_ws_envelope(&mut watcher_ws, |envelope| {
        envelope["kind"] == "created" && envelope["payload"]["request"]["id"] == request_id.as_str()
    })
    .await;
    let watcher_listed = api(
        &http,
        base_url,
        Method::GET,
        "/api/v1/requests",
        &watcher_token,
        None,
    )
    .await;
    let watcher_listed_request = watcher_listed["requests"]
        .as_array()
        .expect("watcher requests array")
        .iter()
        .find(|request| request["id"] == request_id.as_str())
        .unwrap_or_else(|| {
            panic!("request {request_id} missing from watcher list: {watcher_listed}")
        })
        .clone();
    let watcher_fetched_before_decision = api(
        &http,
        base_url,
        Method::GET,
        &format!("/api/v1/requests/{request_id}"),
        &watcher_token,
        None,
    )
    .await;
    assert_eq!(
        watcher_created["payload"]["request"]["recipients"],
        json!([watcher_user_id]),
        "watcher's socket projection should show only the watcher"
    );
    assert_eq!(
        watcher_created["payload"]["request"], watcher_listed_request,
        "watcher sync projection diverges from the HTTP device view"
    );
    assert_eq!(
        watcher_fetched_before_decision["request"], watcher_listed_request,
        "watcher HTTP get projection diverges from the HTTP list view"
    );
    for request_view in [
        &listed_request,
        &fetched_before_decision["request"],
        &ws_created["payload"]["request"],
        &watcher_listed_request,
        &watcher_fetched_before_decision["request"],
        &watcher_created["payload"]["request"],
    ] {
        if let Some(context) = created["request"].get("signing") {
            assert!(
                request_view["request_digest"].is_null(),
                "private projection must omit unsalted digest"
            );
            assert_eq!(&request_view["signing"], context);
            let wire: nod_proto::Request = serde_json::from_value(request_view.clone()).unwrap();
            let context = wire.signing.as_ref().unwrap();
            assert_eq!(
                nod_proto::request_digest_v2(&wire, &context.recipients_commitment).unwrap(),
                context.request_digest
            );
        } else {
            assert_eq!(
                request_view["request_digest"], request_digest,
                "legacy projection carries the v1 digest"
            );
        }
    }

    // Sign and submit the decision with the enrolled key.
    let request_digest = created["request"]["signing"]["request_digest"]
        .as_str()
        .unwrap_or(&request_digest)
        .to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let signed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let text = "smoke approved";
    let payload = decision_signing_payload(DecisionSigningInput {
        request_id: &request_id,
        request_digest: &request_digest,
        option_id: "approve",
        option_kind: &OptionKind::Approve,
        user_id: &user_id,
        device_id: &device_id,
        key_id: &key_id,
        nonce: &nonce,
        signed_at: &signed_at,
        text: Some(text),
    });
    let signature = sign_payload(&key.private_key, payload.as_bytes()).expect("sign decision");
    let resolved = api(
        &http,
        base_url,
        Method::POST,
        &format!("/api/v1/requests/{request_id}/options/approve"),
        &device_token,
        Some(json!({
            "text": text,
            "signature": {
                "key_id": key_id,
                "algorithm": "p256_ecdsa_sha256",
                "nonce": nonce,
                "signed_at": signed_at,
                "request_digest": request_digest,
                "signature": signature
            }
        })),
    )
    .await;
    assert_eq!(
        resolved["request"]["decision"]["signature"]["verified"], true,
        "{resolved}"
    );

    // Resolution lands on the socket and matches the HTTP view again.
    let ws_resolved = next_ws_envelope(&mut ws, |envelope| {
        envelope["kind"] == "resolved"
            && envelope["payload"]["request"]["id"] == request_id.as_str()
    })
    .await;
    assert_eq!(ws_resolved["payload"]["request"]["status"], "resolved");
    let fetched = api(
        &http,
        base_url,
        Method::GET,
        &format!("/api/v1/requests/{request_id}"),
        &device_token,
        None,
    )
    .await;
    assert_eq!(
        ws_resolved["payload"]["request"], fetched["request"],
        "resolved sync projection diverges from the HTTP device view"
    );
    // The non-acting recipient hears about the shared resolution too — a
    // resolution fanned out from a per-user projection would miss them.
    let watcher_resolved = next_ws_envelope(&mut watcher_ws, |envelope| {
        envelope["kind"] == "resolved"
            && envelope["payload"]["request"]["id"] == request_id.as_str()
    })
    .await;
    assert_eq!(watcher_resolved["payload"]["request"]["status"], "resolved");

    // The issuer can read back the signed, verified decision.
    let decision = api(
        &http,
        base_url,
        Method::GET,
        &format!("/api/v1/requests/{request_id}/decision"),
        &issuer_token,
        None,
    )
    .await;
    assert_eq!(decision["decision"]["option_id"], "approve", "{decision}");

    // Success: remove everything the smoke created.
    let issuer_token_id = issuer["id"].as_str().expect("issuer token id");
    for path in [
        format!("/api/v1/admin/devices/{device_id}"),
        format!("/api/v1/admin/devices/{watcher_device_id}"),
        format!("/api/v1/admin/users/{user_id}"),
        format!("/api/v1/admin/users/{watcher_user_id}"),
        format!("/api/v1/admin/channels/{channel_id}"),
        format!("/api/v1/admin/issuer-tokens/{issuer_token_id}"),
    ] {
        api(&http, base_url, Method::DELETE, &path, admin_token, None).await;
    }
}

async fn wait_for_health(http: &Client, base_url: &str) {
    let deadline = tokio::time::Instant::now() + HEALTH_DEADLINE;
    loop {
        match http.get(format!("{base_url}/health")).send().await {
            Ok(response) if response.status() == StatusCode::OK => {
                let body: Value = response.json().await.expect("health body");
                assert_eq!(body["service"], "nod", "{body}");
                return;
            }
            _ if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok(response) => panic!("health returned {}", response.status()),
            Err(err) => panic!("health unreachable at {base_url}: {err}"),
        }
    }
}

async fn api(
    http: &Client,
    base_url: &str,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<Value>,
) -> Value {
    let mut request = http
        .request(method.clone(), format!("{base_url}{path}"))
        .bearer_auth(bearer);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap_or_else(|err| {
        panic!("{method} {path} failed to send: {err}");
    });
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "{method} {path} returned {status}: {text}"
    );
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("{method} {path} returned non-JSON ({err}): {text}"))
    }
}

async fn next_ws_envelope(ws: &mut WsStream, matches: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + WS_DEADLINE;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .expect("timed out waiting for sync envelope");
        let message = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("timed out waiting for sync envelope")
            .expect("sync socket closed")
            .expect("sync socket error");
        let Message::Text(text) = message else {
            continue;
        };
        let envelope: Value = serde_json::from_str(&text).expect("sync envelope is not valid JSON");
        if matches(&envelope) {
            return envelope;
        }
    }
}
