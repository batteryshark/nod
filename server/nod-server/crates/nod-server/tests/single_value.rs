//! Isolated real HTTP + SQLite + shared core signing. No delivery workers run.
use futures_util::StreamExt;
use nod_client_core::{
    build_value_decision_signature, DecisionSigningRequest, DeviceSigner, NodApi, StoredSigningKey,
    SubmitOptionRequest,
};
use serde_json::{json, Value};
use sqlx::Row;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Parent-coordinated browser creates the real baked-in admin card. Finite,
/// localhost-only; never reads environment credentials or runs delivery loops.
#[tokio::test]
#[ignore = "parent-coordinated finite admin browser fixture"]
async fn browser_joint_choice_fixture() {
    tokio::time::timeout(std::time::Duration::from_secs(240), async {
        let directory = tempfile::tempdir().unwrap();
        let mut config = nod_server::Config::with_admin_token("isolated-admin");
        config.database_url = format!("sqlite://{}", directory.path().join("browser.sqlite").display());
        config.data_dir = directory.path().join("data");
        let database_url = config.database_url.clone();
        let state = nod_server::AppState::new(config).await.unwrap();
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:18769").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let _server = Server(tokio::spawn(async move { axum::serve(listener, nod_server::router(state)).await.unwrap() }));
        let http = reqwest::Client::new();
        let code: Value = http.post(format!("{base}/api/v1/admin/users/owner/enrollment-codes")).bearer_auth("isolated-admin").json(&json!({})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let key = StoredSigningKey::generate();
        let enrolled: Value = http.post(format!("{base}/api/v1/enroll")).json(&json!({"code":code["code"],"device_name":"Joint fixture","platform":"linux","signing_key":key.device_signing_key().unwrap()})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        println!("JOINT_FIXTURE_READY base={base} admin_token=isolated-admin title=Joint-value-fixture recipient=owner type=approval choice_ids=small,large default=small; finite240s; delivery_workers=none");
        let id: String = loop {
            if let Some(id) = sqlx::query_scalar("SELECT id FROM requests WHERE title='Joint-value-fixture' AND answer_field_json IS NOT NULL").fetch_optional(&pool).await.unwrap() { break id; }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        };
        let api = NodApi::new(&base, Some(enrolled["token"].as_str().unwrap().into())).unwrap();
        let card = api.get_request(&id).await.unwrap();
        assert_eq!(card.answer_field.as_ref().unwrap().default_value(), &nod_proto::AnswerValue::Choice { choice_id: "small".into() });
        let digest = card.signing.as_ref().unwrap().request_digest.clone();
        let authoritative = api.get_request(&id).await.unwrap();
        let answer = nod_proto::AnswerValue::Choice { choice_id: "large".into() };
        let signature = build_value_decision_signature(&key, DecisionSigningRequest { request: &authoritative, option_id: "approve", text: Some("Joint browser canary notes"), user_id: "owner", device_id: enrolled["device_id"].as_str().unwrap() }, Some(&answer), Some(&digest)).unwrap();
        let receipt = api.submit_option(SubmitOptionRequest { request_id: &id, option_id: "approve", text: Some("Joint browser canary notes"), signature: Some(&signature), answer_value: Some(&answer) }).await.unwrap();
        let decision = receipt.decision.unwrap();
        assert_eq!(decision.answer_value, Some(answer));
        assert_eq!(decision.text.as_deref(), Some("Joint browser canary notes"));
        assert!(decision.signature.as_ref().unwrap().verified);
        println!("JOINT_FIXTURE_VERIFIED request_id={id} decision={}", serde_json::to_string(&decision).unwrap());
        // Keep finite fixture available for parent's browser receipt observation.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    }).await.expect("finite joint browser fixture timed out");
}

#[tokio::test]
async fn signed_choice_http_vertical_slice() {
    tokio::time::timeout(std::time::Duration::from_secs(20), run())
        .await
        .unwrap();
}

async fn run() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = nod_server::Config::with_admin_token("isolated-admin");
    config.database_url = format!(
        "sqlite://{}",
        directory.path().join("value.sqlite").display()
    );
    config.data_dir = directory.path().join("data");
    // Default APNs/NATS are disabled; no main-loop delivery tasks are started.
    let database_url = config.database_url.clone();
    let state = nod_server::AppState::new(config).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, nod_server::router(state))
            .await
            .unwrap()
    }));
    let http = reqwest::Client::new();
    let admin = |path: &str| {
        http.post(format!("{base}{path}"))
            .bearer_auth("isolated-admin")
    };
    let code: Value = admin("/api/v1/admin/users/owner/enrollment-codes")
        .json(&json!({"expires_in_seconds":600}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = StoredSigningKey::generate();
    let enrolled: Value = http.post(format!("{base}/api/v1/enroll")).json(&json!({"code":code["code"],"device_name":"Value fixture","platform":"linux","signing_key":key.device_signing_key().unwrap()})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    let token = enrolled["token"].as_str().unwrap();
    let (mut legacy_socket, _) = tokio_tungstenite::connect_async(format!(
        "{}/api/v1/sync?token={token}",
        base.replace("http://", "ws://")
    ))
    .await
    .unwrap();
    legacy_socket.next().await.unwrap().unwrap(); // authenticated hello
    let field = json!({"kind":"choice","label":"Batch","choices":[{"id":"small","label":"Small"},{"id":"large","label":"Large"}],"default":{"kind":"choice","choice_id":"small"}});
    let create = json!({"title":"Isolated value","recipients":["owner"],"options":[{"id":"apply","label":"Apply","kind":"approve"}],"answer_field":field,"callback_url":"http://127.0.0.1:1/never-sent","idempotency_key":"value-first"});
    let response = admin("/api/v1/requests")
        .json(&create)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    let created: Value = response.json().await.unwrap();
    let id = created["request_id"].as_str().unwrap();
    let event = legacy_socket
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    let event: Value = serde_json::from_str(&event).unwrap();
    assert!(event["payload"]["request"].get("answer_field").is_none());
    assert_eq!(
        event["payload"]["request"]["options"][0]["id"],
        "__nod_value_open"
    );
    let path = format!("{base}/api/v1/requests/{id}");
    let raw: Option<String> =
        sqlx::query_scalar("SELECT answer_field_json FROM requests WHERE id=?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw.unwrap()).unwrap(), field);

    let legacy: Value = http
        .get(&path)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(legacy["request"].get("answer_field").is_none());
    assert_eq!(legacy["request"]["options"][0]["id"], "__nod_value_open");
    assert_eq!(legacy["request"]["signing"]["version"], "nod-request-v3");
    assert!(legacy["request"]["request_digest"].is_null());
    let legacy_list: Value = http
        .get(format!("{base}/api/v1/requests"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(legacy_list["requests"][0].get("answer_field").is_none());
    assert_eq!(
        legacy_list["requests"][0]["options"][0]["id"],
        "__nod_value_open"
    );
    assert_eq!(
        http.get(format!("{path}/decision"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    for option in ["apply", "__nod_value_open"] {
        assert!(!http
            .post(format!("{path}/options/{option}"))
            .bearer_auth(token)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    let api = NodApi::new(&base, Some(token.into())).unwrap();
    let rendered = api.get_request(id).await.unwrap();
    let expected = rendered.signing.as_ref().unwrap().request_digest.clone();
    let authoritative = api.get_request(id).await.unwrap();
    let answer = nod_proto::AnswerValue::Choice {
        choice_id: "large".into(),
    };
    let signature = build_value_decision_signature(
        &key,
        DecisionSigningRequest {
            request: &authoritative,
            option_id: "apply",
            text: Some("  Canary notes  "),
            user_id: "owner",
            device_id: enrolled["device_id"].as_str().unwrap(),
        },
        Some(&answer),
        Some(&expected),
    )
    .unwrap();
    // Failed signed mutations must not burn nonce, revision, journal or callback.
    for mutation in [
        json!({"kind":"choice","choice_id":"small"}),
        json!({"kind":"number","decimal":"4"}),
    ] {
        assert!(!http
            .post(format!("{path}/options/apply"))
            .bearer_auth(token)
            .json(&json!({"answer_value":mutation,"text":"Canary notes","signature":signature}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    assert!(!http
        .post(format!("{path}/options/apply"))
        .bearer_auth(token)
        .json(&json!({"answer_value":answer,"text":"tampered notes","signature":signature}))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM requests WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(revision, 1);
    let nonces: i64 = sqlx::query_scalar("SELECT count(*) FROM decision_nonces")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(nonces, 0);
    let callbacks: i64 = sqlx::query_scalar("SELECT count(*) FROM callback_deliveries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(callbacks, 0);
    let receipt = api
        .submit_option(SubmitOptionRequest {
            request_id: id,
            option_id: "apply",
            text: Some("Canary notes"),
            signature: Some(&signature),
            answer_value: Some(&answer),
        })
        .await
        .unwrap();
    let decision = receipt.decision.unwrap();
    assert_eq!(decision.answer_value, Some(answer.clone()));
    assert_eq!(decision.text.as_deref(), Some("Canary notes"));
    assert!(decision.signature.as_ref().unwrap().verified);
    let event = legacy_socket
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(!event.contains("Canary notes") && !event.contains("choice_id"));
    assert_eq!(
        decision
            .signature
            .as_ref()
            .unwrap()
            .payload_version
            .as_deref(),
        Some("nod-decision-v2")
    );
    let fetched = api.get_request(id).await.unwrap();
    assert_eq!(fetched.decision.as_ref().unwrap(), &decision);
    let legacy_receipt: Value = http
        .get(&path)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let legacy_text = serde_json::to_string(&legacy_receipt).unwrap();
    assert!(!legacy_text.contains("Canary notes") && !legacy_text.contains("choice_id"));
    let event = sqlx::query("SELECT envelope_json,private_detail_json FROM lifecycle_events WHERE request_id=? AND kind='nod.answer.recorded'").bind(id).fetch_one(&pool).await.unwrap();
    let envelope: String = event.get("envelope_json");
    assert!(
        !envelope.contains("answer_value")
            && !envelope.contains("Canary notes")
            && !envelope.contains("large")
    );
    let detail: Value = serde_json::from_str(event.get("private_detail_json")).unwrap();
    assert_eq!(detail["decision"]["answer_value"], json!(answer));
    assert_eq!(detail["decision"]["text"], "Canary notes");
    let callback: String =
        sqlx::query_scalar("SELECT payload_json FROM callback_deliveries WHERE request_id=?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        callback.contains("Canary notes")
            && callback.contains("large")
            && callback.contains("nod-decision-v2")
    );
    let changed = {
        let mut c = create.clone();
        c["answer_field"]["default"]["choice_id"] = json!("large");
        c
    };
    assert_eq!(
        admin("/api/v1/requests")
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
}
