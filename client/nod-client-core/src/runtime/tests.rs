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
    preferences: Arc<Mutex<crate::models::DeviceNotificationPreferences>>,
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
    async fn start() -> Self {
        let (messages, _) = broadcast::channel(16);
        let fixture = Fixture {
            requests: Arc::new(Mutex::new(vec![request("initial")])),
            messages,
            posts: Arc::new(Mutex::new(vec![])),
            connections: Arc::new(AtomicUsize::new(0)),
            fail_snapshot: Arc::new(AtomicBool::new(false)),
            preferences: Arc::new(Mutex::new(Default::default())),
        };
        let app = Router::new()
            .route("/api/v1/enroll", axum::routing::post(||async {Json(json!({"device_id":"device","user_id":"owner","user_name":"Owner","token":"target-token","channels":[],"devices":[]}))}))
            .route("/api/v1/users/me", get(|State(state):State<Fixture>| async move {if state.fail_snapshot.load(Ordering::SeqCst) {return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":"temporarily unavailable"})));} (StatusCode::OK,Json(json!({"user":{"id":"owner","name":"Owner","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}, "current_device":device_with_preferences(&state).await, "notification_delivery":{"mode":"websocket"}})))}))
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
) -> Json<Value> {
    state.posts.lock().await.push(body);
    let mut requests = state.requests.lock().await;
    let request = requests
        .iter_mut()
        .find(|request| request.id == id)
        .unwrap();
    request.status = RequestStatus::Resolved;
    request.updated_at = chrono::Utc::now();
    Json(json!({"request":request}))
}

async fn socket(State(state): State<Fixture>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
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
    request.request_digest = Some(nod_proto::request_digest(&request).unwrap());
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

#[tokio::test]
async fn notification_submission_fetches_and_signs_on_its_own_server_without_switching() {
    let server = TestServer::start().await;
    let (mut runtime, _directory, _messages) = runtime(&server, true).await;
    let selected_before = runtime.state().await.selected_server_id;
    let result = runtime
        .submit_request_option(SubmitRequestOptionParams {
            server_id: profile_id_for(&server.url),
            request_id: "initial".into(),
            option_id: "dismiss".into(),
            text: None,
        })
        .await
        .unwrap();
    assert_eq!(result.status, RequestStatus::Resolved);
    assert_eq!(runtime.state().await.selected_server_id, selected_before);
    assert!(
        server.fixture.posts.lock().await[0]["signature"]["signature"]
            .as_str()
            .is_some()
    );
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
