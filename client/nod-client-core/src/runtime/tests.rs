use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{
    extract::ws::Message,
    extract::{Path, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use super::*;
use crate::{
    api::profile_id_for,
    models::{DecisionResolution, RequestStatus, ServerProfile, SyncPhase},
    signing::StoredSigningKey,
};

#[derive(Clone)]
struct Fixture {
    requests: Arc<Mutex<Vec<Request>>>,
    messages: broadcast::Sender<Message>,
    posts: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
    fail_snapshot: Arc<AtomicBool>,
    gateway_failures: Arc<AtomicUsize>,
    snapshot_gets: Arc<AtomicUsize>,
    lost_submit_response: Arc<AtomicBool>,
    preferences: Arc<Mutex<crate::models::DeviceNotificationPreferences>>,
    sync_query_rejections: Arc<AtomicUsize>,
    /// When set, a targeted GET returns this request (with a valid digest of
    /// its own) instead of the stored one: a server swapping content between
    /// display and submit.
    swapped_request: Arc<Mutex<Option<Request>>>,
}

struct TestServer {
    url: String,
    fixture: Fixture,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestServer {
    // fetch_update supports Docker's Rust 1.94; try_update requires newer Rust.
    #[allow(deprecated)]
    async fn start() -> Self {
        let (messages, _) = broadcast::channel(16);
        let fixture = Fixture {
            requests: Arc::new(Mutex::new(vec![request("initial")])),
            messages,
            posts: Arc::new(Mutex::new(vec![])),
            connections: Arc::new(AtomicUsize::new(0)),
            fail_snapshot: Arc::new(AtomicBool::new(false)),
            gateway_failures: Arc::new(AtomicUsize::new(0)),
            snapshot_gets: Arc::new(AtomicUsize::new(0)),
            lost_submit_response: Arc::new(AtomicBool::new(false)),
            preferences: Arc::new(Mutex::new(Default::default())),
            sync_query_rejections: Arc::new(AtomicUsize::new(0)),
            swapped_request: Arc::new(Mutex::new(None)),
        };
        let app = Router::new()
            .route("/api/v1/enroll", axum::routing::post(||async {Json(json!({"device_id":"device","user_id":"owner","user_name":"Owner","token":"target-token","channels":[],"devices":[]}))}))
            .route("/api/v1/users/me", get(|State(state):State<Fixture>| async move {
                state.snapshot_gets.fetch_add(1, Ordering::SeqCst);
                if state.gateway_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {return (StatusCode::BAD_GATEWAY,Json(json!({"error":"sensitive gateway body must not escape"})));}
                if state.fail_snapshot.load(Ordering::SeqCst) {return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":"temporarily unavailable"})));} (StatusCode::OK,Json(json!({"user":{"id":"owner","name":"Owner","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}, "current_device":device_with_preferences(&state).await, "notification_delivery":{"mode":"websocket"}})))}))
            .route("/api/v1/users/me/devices", get(|| async {Json(json!({"devices":[device()]}))}))
            .route("/api/v1/channels", get(|| async {Json(json!({"channels":[{"id":"other","name":"Other","emoji":"","subscribed":true,"created_at":"2026-01-01T00:00:00Z"}]}))}))
            .route("/api/v1/requests", get(|State(state): State<Fixture>| async move {Json(json!({"requests":state.requests.lock().await.clone(),"next_cursor":null}))}))
            .route("/api/v1/requests/{id}", get(get_request))
            .route("/api/v1/requests/{id}/options/{option}", axum::routing::post(submit))
            .route("/api/v1/devices/me/notification-preferences", axum::routing::put(|State(state): State<Fixture>, Json(preferences):Json<crate::models::DeviceNotificationPreferences>| async move {*state.preferences.lock().await = preferences; Json(json!({"ok":true}))}))
            .route("/api/v1/devices/me/push-token", axum::routing::put(|State(state): State<Fixture>, Json(body):Json<Value>| async move {state.posts.lock().await.push(body); Json(json!({"ok":true}))}))
            .route("/api/v1/sync", get(socket))
            .with_state(fixture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, fixture, task }
    }
}

async fn get_request(
    State(state): State<Fixture>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer target-token")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let requests = state.requests.lock().await;
    assert_eq!(
        headers
            .get("Nod-Request-Contract")
            .and_then(|v| v.to_str().ok()),
        Some("single-value-v1")
    );
    if let Some(swapped) = state
        .swapped_request
        .lock()
        .await
        .as_ref()
        .filter(|swapped| swapped.id == id)
    {
        return Ok(Json(json!({"request":swapped})));
    }
    let request = requests
        .iter()
        .find(|request| request.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({"request":request})))
}

async fn submit(
    State(state): State<Fixture>,
    Path((id, _)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    state.posts.lock().await.push(body);
    let mut requests = state.requests.lock().await;
    let request = requests
        .iter_mut()
        .find(|request| request.id == id)
        .unwrap();
    request.status = RequestStatus::Resolved;
    request.updated_at = chrono::Utc::now();
    if state.lost_submit_response.load(Ordering::SeqCst) {
        return (StatusCode::BAD_GATEWAY, "sensitive lost response body").into_response();
    }
    Json(json!({"request":request})).into_response()
}

async fn socket(
    State(state): State<Fixture>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    // The server contract: the device token arrives only as a bearer header on
    // the upgrade request, never as a query parameter.
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer target-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if uri.query().is_some_and(|query| !query.is_empty()) {
        state.sync_query_rejections.fetch_add(1, Ordering::SeqCst);
        return StatusCode::BAD_REQUEST.into_response();
    }
    upgrade.on_upgrade(move |mut socket| async move {
        let mut messages = state.messages.subscribe();
        state.connections.fetch_add(1, Ordering::SeqCst);
        loop {
            tokio::select! {
                message = messages.recv() => match message {
                    Ok(message) => {if socket.send(message).await.is_err() { break; }},
                    Err(_) => break,
                },
                message = socket.recv() => match message {
                    Some(Ok(Message::Ping(bytes))) => {if socket.send(Message::Pong(bytes)).await.is_err() {break;}},
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {},
                }
            }
        }
    })
    .into_response()
}

async fn device_with_preferences(state: &Fixture) -> Value {
    let mut device = device();
    device["notification_preferences"] =
        serde_json::to_value(state.preferences.lock().await.clone()).unwrap();
    device
}

fn device() -> Value {
    json!({"id":"device","user_id":"owner","name":"Test","platform":"linux","push_provider":null,"has_push_token":false,"notification_sound":"default","last_seen_at":"2026-01-01T00:00:00Z","created_at":"2026-01-01T00:00:00Z","is_current":true})
}

fn request(id: &str) -> Request {
    let now = chrono::Utc::now();
    let mut request = Request {
        answer_field: None,
        id: id.into(),
        request_id: id.into(),
        channel_id: "other".into(),
        recipients: vec!["owner".into()],
        decision_resolution: DecisionResolution::Shared,
        title: id.into(),
        summary: String::new(),
        body_markdown: String::new(),
        fields: vec![],
        links: vec![],
        image_url: None,
        notification: Default::default(),
        dedupe_key: None,
        expires_at: None,
        status: RequestStatus::Pending,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        decision: None,
        decisions: vec![],
        callback_url: None,
        options: vec![],
        request_digest: None,
        signing: None,
    };
    request.request_digest =
        Some(nod_proto::request_digest_legacy_line_snapshot(&request).unwrap());
    request
}

async fn runtime(
    server: &TestServer,
    selected_other: bool,
) -> (
    NodClientRuntime,
    tempfile::TempDir,
    mpsc::Receiver<NodClientMessage>,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::test_store(directory.path().join("client-core.json"));
    let id = profile_id_for(&server.url);
    let profile = ServerProfile {
        id: id.clone(),
        credential_id: None,
        name: "Target".into(),
        base_url_string: server.url.clone(),
        device_name: "Test".into(),
        device_id: Some("device".into()),
        user_id: Some("owner".into()),
        user_name: Some("Owner".into()),
    };
    let mut config = PersistedConfig {
        servers: vec![profile],
        selected_server_id: Some(id.clone()),
        ..Default::default()
    };
    config
        .insecure_tokens
        .insert(id.clone(), "target-token".into());
    config
        .insecure_signing_keys
        .insert(id, StoredSigningKey::generate());
    if selected_other {
        let other_id = profile_id_for("http://127.0.0.1:1");
        let mut other = config.servers[0].clone();
        other.id = other_id.clone();
        other.base_url_string = "http://127.0.0.1:1".into();
        config.servers.push(other);
        config
            .insecure_tokens
            .insert(other_id.clone(), "offline-token".into());
        config.selected_server_id = Some(other_id);
    }
    store.save(config).await.unwrap();
    let (tx, rx) = mpsc::channel(256);
    (
        NodClientRuntime::with_store(tx, SignerBackend::Software, store)
            .await
            .unwrap(),
        directory,
        rx,
    )
}

async fn until(runtime: &NodClientRuntime, predicate: impl Fn(&ClientState) -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if predicate(&runtime.state().await) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn targeted_get_rpc_is_read_only_and_does_not_switch_selection() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, true).await;
    let before = serde_json::to_value(runtime.state().await).unwrap();
    let result = runtime
        .handle_method(
            "get_request",
            json!({
                "server_id": profile_id_for(&server.url), "request_id": "initial"
            }),
        )
        .await
        .unwrap();
    assert_eq!(result["request"]["id"], "initial");
    assert_eq!(serde_json::to_value(runtime.state().await).unwrap(), before);
    assert!(server.fixture.posts.lock().await.is_empty());
    assert_eq!(server.fixture.snapshot_gets.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn device_preferences_apply_before_notification_candidates_without_hiding_inbox() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, mut messages) = runtime(&server, false).await;
    let params = serde_json::from_value(json!({"server_id":profile_id_for(&server.url),"hide_content":true,"muted_channels":[],"snoozed_until":null})).unwrap();
    runtime
        .set_device_notification_preferences(params)
        .await
        .unwrap();
    while messages.try_recv().is_ok() {}
    server
        .fixture
        .requests
        .lock()
        .await
        .push(request("hidden-preview"));
    runtime.refresh().await.unwrap();
    let candidate = std::iter::from_fn(|| messages.try_recv().ok())
        .find_map(|message| match message {
            NodClientMessage::NotificationCandidate { request, .. } => Some(request),
            _ => None,
        })
        .unwrap();
    assert!(candidate.notification.redact);
    assert!(
        !runtime
            .state()
            .await
            .requests
            .iter()
            .find(|request| request.id == "hidden-preview")
            .unwrap()
            .notification
            .redact
    );
    let params = serde_json::from_value(json!({"server_id":profile_id_for(&server.url),"hide_content":false,"muted_channels":["other"],"snoozed_until":null})).unwrap();
    runtime
        .set_device_notification_preferences(params)
        .await
        .unwrap();
    while messages.try_recv().is_ok() {}
    server.fixture.requests.lock().await.push(request("muted"));
    runtime.refresh().await.unwrap();
    assert!(!std::iter::from_fn(|| messages.try_recv().ok())
        .any(|message| matches!(message, NodClientMessage::NotificationCandidate { .. })));
    assert!(runtime
        .state()
        .await
        .requests
        .iter()
        .any(|request| request.id == "muted"));
}

#[tokio::test]
async fn preference_setter_targets_explicit_server_without_switching_selection() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, true).await;
    let selected = runtime.state().await.selected_server_id;
    runtime
        .set_device_notification_preferences(DeviceNotificationPreferenceParams {
            server_id: Some(profile_id_for(&server.url)),
            preferences: crate::models::DeviceNotificationPreferences {
                hide_content: true,
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert!(server.fixture.preferences.lock().await.hide_content);
    assert_eq!(runtime.state().await.selected_server_id, selected);
}

#[tokio::test]
async fn committed_enrollment_succeeds_when_initial_inbox_fetch_fails() {
    let server = TestServer::start().await;
    server.fixture.fail_snapshot.store(true, Ordering::SeqCst);
    let directory = tempfile::tempdir().unwrap();
    let store = Store::test_store(directory.path().join("client-core.json"));
    let (tx, _messages) = mpsc::channel(32);
    let mut runtime = NodClientRuntime::with_store(tx, SignerBackend::Software, store.clone())
        .await
        .unwrap();
    let state = runtime
        .enroll(EnrollParams {
            base_url: server.url.clone(),
            device_name: "Test".into(),
            code: "ABCDEFGH".into(),
            notification_sound: None,
            platform: None,
            native_app_id: None,
            push_provider: None,
            push_token: None,
            attestation: None,
        })
        .await
        .unwrap();
    assert!(state.is_registered);
    assert!(state
        .last_error
        .as_deref()
        .unwrap()
        .contains("Enrollment completed"));
    assert_eq!(store.load().await.unwrap().servers.len(), 1);
    server.fixture.fail_snapshot.store(false, Ordering::SeqCst);
    runtime.connect_sync().await.unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Current).await;
    runtime.disconnect_sync().await;
}

async fn stored_digest(server: &TestServer, id: &str) -> String {
    let requests = server.fixture.requests.lock().await;
    let request = requests.iter().find(|request| request.id == id).unwrap();
    crate::signing::asserted_request_digest(request)
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn notification_submission_fetches_and_signs_on_its_own_server_without_switching() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, true).await;
    let selected_before = runtime.state().await.selected_server_id;
    let rendered = stored_digest(&server, "initial").await;
    let result = runtime
        .submit_request_option(SubmitRequestOptionParams {
            answer_value: None,
            expected_request_digest: rendered.clone(),
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "dismiss".into(),
            text: None,
        })
        .await
        .unwrap();
    assert_eq!(result.status, RequestStatus::Resolved);
    assert_eq!(runtime.state().await.selected_server_id, selected_before);
    let posts = server.fixture.posts.lock().await;
    assert!(posts[0]["signature"]["signature"].as_str().is_some());
    assert_eq!(posts[0]["signature"]["request_digest"], rendered);
}

#[tokio::test]
async fn ordinary_submission_without_the_rendered_digest_is_refused_before_any_post() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    runtime.refresh().await.unwrap();
    for expected in ["", "   ", "not-the-rendered-digest"] {
        let error = runtime
            .submit_request_option(SubmitRequestOptionParams {
                answer_value: None,
                expected_request_digest: expected.into(),
                server_id: profile_id_for(&server.url),
                request_id: "initial".into(),
                option_id: "dismiss".into(),
                text: None,
            })
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(error, crate::signing::REQUEST_CHANGED_ERROR);
    }
    let missing_field = runtime
        .handle_method(
            "submit_option",
            json!({"request_id": "initial", "option_id": "dismiss"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(missing_field.contains("expected_request_digest"));
    assert!(server.fixture.posts.lock().await.is_empty());
    assert_eq!(
        runtime.state().await.requests[0].status,
        RequestStatus::Pending
    );
}

#[tokio::test]
async fn server_swapping_content_at_submit_time_is_refused_before_any_post() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    runtime.refresh().await.unwrap();
    let rendered = stored_digest(&server, "initial").await;
    // The server serves a different, self-consistent request for the same id
    // once the device comes back to submit.
    let mut swapped = request("initial");
    swapped.title = "Transfer the production signing key".into();
    swapped.request_digest =
        Some(nod_proto::request_digest_legacy_line_snapshot(&swapped).unwrap());
    assert_ne!(swapped.request_digest.as_deref(), Some(rendered.as_str()));
    *server.fixture.swapped_request.lock().await = Some(swapped);
    let error = runtime
        .submit_request_option(SubmitRequestOptionParams {
            answer_value: None,
            expected_request_digest: rendered,
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "dismiss".into(),
            text: None,
        })
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, crate::signing::REQUEST_CHANGED_ERROR);
    assert!(server.fixture.posts.lock().await.is_empty());
}

#[tokio::test]
async fn submission_signs_the_locally_rendered_copy() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    runtime.refresh().await.unwrap();
    let rendered = stored_digest(&server, "initial").await;
    // The submit-time fetch asserts the rendered digest but carries content
    // that does not hash to it. The local copy is what gets signed and the
    // fetched content never reaches the signer, so the digest recompute
    // over the rendered copy still passes and the post carries that digest.
    let mut lying = request("initial");
    lying.title = "Transfer the production signing key".into();
    lying.request_digest = Some(rendered.clone());
    *server.fixture.swapped_request.lock().await = Some(lying);
    let result = runtime
        .submit_request_option(SubmitRequestOptionParams {
            answer_value: None,
            expected_request_digest: rendered.clone(),
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "dismiss".into(),
            text: None,
        })
        .await
        .unwrap();
    assert_eq!(result.status, RequestStatus::Resolved);
    let posts = server.fixture.posts.lock().await;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0]["signature"]["request_digest"], rendered);
}

#[tokio::test]
async fn sync_upgrade_sends_the_device_token_as_a_bearer_header_only() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    runtime.connect_sync().await.unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Current).await;
    assert_eq!(server.fixture.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        server.fixture.sync_query_rejections.load(Ordering::SeqCst),
        0
    );
    runtime.disconnect_sync().await;
}

#[tokio::test]
async fn missing_local_token_is_not_reported_as_server_revocation() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, mut messages) = runtime(&server, false).await;
    runtime.persisted.lock().await.insecure_tokens.clear();
    let error = runtime.connect_sync().await.unwrap_err();
    assert!(error.to_string().contains("Unlock the credential store"));
    assert_eq!(server.fixture.connections.load(Ordering::SeqCst), 0);
    assert!(!std::iter::from_fn(|| messages.try_recv().ok())
        .any(|message| matches!(message, NodClientMessage::AuthRevoked {})));
}

#[tokio::test]
async fn push_token_rotation_reaches_healthy_server_after_an_offline_server() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, true).await;
    runtime.persisted.lock().await.servers.reverse();
    let error = runtime
        .register_push_token(RegisterPushTokenParams {
            provider: "apns".into(),
            native_app_id: "example.app".into(),
            token: "rotated-token".into(),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("1 of 2 servers"));
    assert_eq!(
        server.fixture.posts.lock().await[0]["token"],
        "rotated-token"
    );
}

#[tokio::test]
async fn reconnect_and_resync_reconcile_without_manual_refresh() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    runtime.connect_sync().await.unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Current).await;
    runtime.connect_sync().await.unwrap();
    assert_eq!(server.fixture.connections.load(Ordering::SeqCst), 1);
    server.fixture.messages.send(Message::Close(None)).unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Offline).await;
    server.fixture.requests.lock().await.push(request("missed"));
    until(&runtime, |state| {
        state.sync_phase == SyncPhase::Current
            && state.requests.iter().any(|request| request.id == "missed")
    })
    .await;
    assert!(server.fixture.connections.load(Ordering::SeqCst) >= 2);
    assert!(runtime.state().await.last_synced_at.is_some());
    server.fixture.requests.lock().await.push(request("lagged"));
    server
        .fixture
        .messages
        .send(Message::Text(
            json!({"kind":"resync_required","at":chrono::Utc::now(),"payload":{}})
                .to_string()
                .into(),
        ))
        .unwrap();
    until(&runtime, |state| {
        state.sync_phase == SyncPhase::Current
            && state.requests.iter().any(|request| request.id == "lagged")
    })
    .await;
    runtime.disconnect_sync().await;
}

#[tokio::test]
async fn gateway_get_recovers_then_exhausts_without_losing_cards_or_replaying_alerts() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, mut messages) = runtime(&server, false).await;
    server.fixture.gateway_failures.store(1, Ordering::SeqCst);
    runtime.refresh().await.unwrap();
    assert_eq!(server.fixture.snapshot_gets.load(Ordering::SeqCst), 2);
    while messages.try_recv().is_ok() {}
    server.fixture.gateway_failures.store(10, Ordering::SeqCst);
    let before = runtime.state().await;
    let error = runtime.refresh().await.unwrap_err().to_string();
    assert!(error.contains("GET /api/v1/users/me"));
    assert!(error.contains("502 Bad Gateway (3 attempt(s))"));
    assert!(!error.contains("sensitive"));
    assert_eq!(server.fixture.snapshot_gets.load(Ordering::SeqCst), 5);
    assert_eq!(runtime.state().await.requests, before.requests);
    assert_eq!(
        runtime.state().await.pending_counts_by_channel,
        before.pending_counts_by_channel
    );
    server.fixture.gateway_failures.store(0, Ordering::SeqCst);
    runtime.refresh().await.unwrap();
    assert!(!std::iter::from_fn(|| messages.try_recv().ok())
        .any(|m| matches!(m, NodClientMessage::NotificationCandidate { .. })));
}

#[tokio::test]
async fn lost_decision_response_posts_once_and_reconciles_with_get() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    server
        .fixture
        .lost_submit_response
        .store(true, Ordering::SeqCst);
    let rendered = stored_digest(&server, "initial").await;
    let error = runtime
        .submit_request_option(SubmitRequestOptionParams {
            answer_value: None,
            expected_request_digest: rendered,
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "dismiss".into(),
            text: None,
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("POST /api/v1/requests/initial/options/dismiss"));
    assert!(error.contains("1 attempt(s)"));
    assert!(!error.contains("sensitive"));
    assert_eq!(server.fixture.posts.lock().await.len(), 1);
    assert_eq!(
        runtime.state().await.requests[0].status,
        RequestStatus::Resolved
    );
}

#[tokio::test]
async fn lost_value_response_keeps_one_post_and_reconciles_read_only() {
    let server = TestServer::start().await;
    let digest;
    {
        let mut cards = server.fixture.requests.lock().await;
        let card = &mut cards[0];
        card.answer_field = Some(nod_proto::AnswerField::Number {
            label: "Count".into(),
            default: nod_proto::AnswerValue::Number {
                decimal: "4".into(),
            },
        });
        card.options.push(crate::models::RequestOption {
            id: "apply".into(),
            label: "Apply".into(),
            kind: crate::models::OptionKind::Approve,
            style: "default".into(),
            requires_text: false,
            text_placeholder: None,
            destructive: false,
            foreground: false,
        });
        digest = nod_proto::value_signing::value_request_digest(
            card,
            &"a".repeat(64),
            card.answer_field.as_ref().unwrap(),
        )
        .unwrap();
        card.signing = Some(nod_proto::RequestSigningContext {
            version: nod_proto::value_signing::VALUE_REQUEST_DIGEST_VERSION.into(),
            recipients_commitment: "a".repeat(64),
            request_digest: digest.clone(),
        });
        card.request_digest = None;
    }
    let (mut runtime, _directory, _messages) = runtime(&server, false).await;
    server
        .fixture
        .lost_submit_response
        .store(true, Ordering::SeqCst);
    let result = runtime
        .submit_request_option(SubmitRequestOptionParams {
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "apply".into(),
            text: Some("notes".into()),
            answer_value: Some(nod_proto::AnswerValue::Number {
                decimal: "5".into(),
            }),
            expected_request_digest: digest,
        })
        .await;
    assert!(result.is_err());
    let posts = server.fixture.posts.lock().await;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0]["answer_value"]["decimal"], "5");
    assert_eq!(posts[0]["signature"]["payload_version"], "nod-decision-v2");
    assert_eq!(
        runtime.state().await.requests[0].status,
        RequestStatus::Resolved
    );
}

#[tokio::test]
async fn revoked_registration_clears_requests_and_delivered_notifications() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, mut messages) = runtime(&server, false).await;
    runtime.connect_sync().await.unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Current).await;
    server.fixture.messages.send(Message::Text(json!({"kind":"device_revoked","at":chrono::Utc::now(),"payload":{"device_id":"device"}}).to_string().into())).unwrap();
    until(&runtime, |state| state.sync_phase == SyncPhase::Revoked).await;
    assert!(runtime.state().await.requests.is_empty());
    let removed = std::iter::from_fn(||messages.try_recv().ok()).any(|message|matches!(message, NodClientMessage::NotificationRemoved {server_id,request_id} if server_id == profile_id_for(&server.url) && request_id == "initial"));
    assert!(
        removed,
        "revocation removes previously delivered request notifications"
    );
    runtime.disconnect_sync().await;
}

#[test]
fn migration_keeps_hardware_and_keyring_credential_account_identity() {
    let mut config = PersistedConfig::default();
    config.servers.push(ServerProfile {
        id: "legacy".into(),
        credential_id: None,
        name: "A".into(),
        base_url_string: "https://nod.example/a".into(),
        device_name: "D".into(),
        device_id: None,
        user_id: None,
        user_name: None,
    });
    config.selected_server_id = Some("legacy".into());
    assert!(migrate_profile_ids(&mut config));
    assert_eq!(config.servers[0].credential_id(), "legacy");
    assert_eq!(
        config.selected_server_id.as_deref(),
        Some(config.servers[0].id.as_str())
    );
    assert!(!migrate_profile_ids(&mut config));
}
