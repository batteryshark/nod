use std::time::Duration;

use futures_util::StreamExt;
use nod_client_core::{
    build_decision_signature, DecisionSigningRequest, DeviceSigner, StoredSigningKey,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::support::TestApp;
use super::support::*;

const ADMIN: Option<&str> = Some("admin-test-token");

async fn signed_device(app: &TestApp) -> (StoredSigningKey, String, String) {
    let signer = StoredSigningKey::generate();
    let (_, code) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            ADMIN,
            Some(json!({})),
        )
        .await;
    let (status, enrolled) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": code["code"], "device_name": "Signed client", "platform": "linux",
                "signing_key": signer.device_signing_key().unwrap(),
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
    (
        signer,
        enrolled["device_id"].as_str().unwrap().to_string(),
        enrolled["token"].as_str().unwrap().to_string(),
    )
}

fn approval(title: &str) -> Value {
    json!({"title": title, "options": [{"id":"approve", "label":"Approve", "kind":"approve"}]})
}

#[tokio::test]
async fn invalid_options_never_leave_partial_requests() {
    let app = TestApp::new().await;
    for invalid in [
        json!({"id":"bad option", "label":"Reject", "kind":"reject"}),
        json!({"id":"reject", "label":"", "kind":"reject"}),
        json!({"id":"approve", "label":"Duplicate", "kind":"reject"}),
    ] {
        let mut body = approval("Must roll back");
        body["options"].as_array_mut().unwrap().push(invalid);
        let (status, response) = app
            .request(Method::POST, "/api/v1/requests", ADMIN, Some(body))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM requests")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn simultaneous_dedupe_retries_return_one_complete_request() {
    let app = TestApp::new().await;
    let mut body = approval("Retry safely");
    body["dedupe_key"] = json!("concurrent-create");
    body["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"reject", "label":"Reject", "kind":"reject"}));
    let (first, second) = tokio::join!(
        app.request(Method::POST, "/api/v1/requests", ADMIN, Some(body.clone())),
        app.request(Method::POST, "/api/v1/requests", ADMIN, Some(body)),
    );
    assert_eq!((first.0, second.0), (StatusCode::OK, StatusCode::OK));
    assert_eq!(first.1["request_id"], second.1["request_id"]);
    assert_eq!(first.1["request"]["options"].as_array().unwrap().len(), 2);
    assert_eq!(second.1["request"]["options"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn cancellation_and_last_per_user_decision_have_one_winner() {
    let app = TestApp::new().await;
    let (_, token) = app.enroll_device("Race client", "linux").await;
    for _ in 0..16 {
        let mut body = approval("Terminal race");
        body["decision_resolution"] = json!("per_user");
        body["recipients"] = json!(["owner"]);
        let (_, created) = app
            .request(Method::POST, "/api/v1/requests", ADMIN, Some(body))
            .await;
        let id = created["request_id"].as_str().unwrap();
        let cancel = format!("/api/v1/requests/{id}/cancel");
        let decide = format!("/api/v1/requests/{id}/options/approve");
        let (cancelled, decided) = tokio::join!(
            app.request(Method::POST, &cancel, ADMIN, None),
            app.request(Method::POST, &decide, Some(&token), Some(json!({}))),
        );
        assert!(
            matches!(
                (cancelled.0, decided.0),
                (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
            ),
            "cancel={cancelled:?}, decide={decided:?}"
        );
    }
}

#[tokio::test]
async fn client_signer_accepts_private_http_and_websocket_projections() {
    let app = TestApp::new().await;
    app.create_user("other", "Other recipient").await;
    let (signer, device_id, token) = signed_device(&app).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, _) = connect_async(format!("ws://{address}/api/v1/sync?token={token}"))
        .await
        .unwrap();
    socket.next().await.unwrap().unwrap();
    for resolution in ["shared", "per_user"] {
        for transport in ["http", "websocket"] {
            let mut body = approval("Multiple private recipients");
            body["recipients"] = json!(["owner", "other"]);
            body["decision_resolution"] = json!(resolution);
            let (_, created) = app
                .request(Method::POST, "/api/v1/requests", ADMIN, Some(body))
                .await;
            let id = created["request_id"].as_str().unwrap();
            let request_value = if transport == "http" {
                app.request(
                    Method::GET,
                    &format!("/api/v1/requests/{id}"),
                    Some(&token),
                    None,
                )
                .await
                .1["request"]
                    .clone()
            } else {
                loop {
                    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                    if let Message::Text(text) = message {
                        let envelope: Value = serde_json::from_str(&text).unwrap();
                        if envelope["kind"] == "created"
                            && envelope["payload"]["request"]["id"] == id
                        {
                            break envelope["payload"]["request"].clone();
                        }
                    }
                }
            };
            assert_eq!(request_value["recipients"], json!(["owner"]));
            assert!(request_value["request_digest"].is_null());
            assert!(!request_value
                .to_string()
                .contains(created["request"]["request_digest"].as_str().unwrap()));
            let request: nod_proto::Request = serde_json::from_value(request_value).unwrap();
            let signature = build_decision_signature(
                &signer,
                DecisionSigningRequest {
                    request: &request,
                    option_id: "approve",
                    text: None,
                    user_id: "owner",
                    device_id: &device_id,
                },
            )
            .unwrap();
            let (status, response) = app
                .request(
                    Method::POST,
                    &format!("/api/v1/requests/{id}/options/approve"),
                    Some(&token),
                    Some(json!({"signature":signature})),
                )
                .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "{resolution}/{transport}: {response}"
            );
            assert_eq!(
                response["request"]["decision"]["signature"]["verified"],
                true
            );
            assert!(!response
                .to_string()
                .contains(created["request"]["request_digest"].as_str().unwrap()));
            let (_, decision) = app
                .request(
                    Method::GET,
                    &format!("/api/v1/requests/{id}/decision"),
                    Some(&token),
                    None,
                )
                .await;
            assert!(decision["request_digest"].is_null());
        }
    }
    server.abort();
}

#[tokio::test]
async fn revocation_closes_a_socket_without_client_cooperation() {
    let app = TestApp::new().await;
    let (device_id, token) = app.enroll_device("Uncooperative", "linux").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, _) = connect_async(format!("ws://{address}/api/v1/sync?token={token}"))
        .await
        .unwrap();
    socket.next().await.unwrap().unwrap();
    let (status, _) = app
        .request(
            Method::DELETE,
            &format!("/api/v1/admin/devices/{device_id}"),
            ADMIN,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    app.request(
        Method::POST,
        "/api/v1/requests",
        ADMIN,
        Some(approval("Private after revocation")),
    )
    .await;
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    assert!(matches!(message, None | Some(Ok(Message::Close(_)))));
    server.abort();
}

#[tokio::test]
async fn deleted_users_and_revoked_devices_keep_verifiable_evidence() {
    let app = TestApp::new().await;
    app.create_user("other", "Other recipient").await;
    let (signer, device_id, token) = signed_device(&app).await;
    let mut body = approval("Durable receipt");
    body["recipients"] = json!(["owner", "other"]);
    body["decision_resolution"] = json!("per_user");
    let (_, created) = app
        .request(Method::POST, "/api/v1/requests", ADMIN, Some(body))
        .await;
    let id = created["request_id"].as_str().unwrap();
    let (_, view) = app
        .request(
            Method::GET,
            &format!("/api/v1/requests/{id}"),
            Some(&token),
            None,
        )
        .await;
    let request: nod_proto::Request = serde_json::from_value(view["request"].clone()).unwrap();
    let signature = build_decision_signature(
        &signer,
        DecisionSigningRequest {
            request: &request,
            option_id: "approve",
            text: None,
            user_id: "owner",
            device_id: &device_id,
        },
    )
    .unwrap();
    let (status, _) = app
        .request(
            Method::POST,
            &format!("/api/v1/requests/{id}/options/approve"),
            Some(&token),
            Some(json!({"signature":signature})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    app.request(
        Method::DELETE,
        &format!("/api/v1/admin/devices/{device_id}"),
        ADMIN,
        None,
    )
    .await;
    let (status, _) = app
        .request(Method::DELETE, "/api/v1/admin/users/owner", ADMIN, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = app
        .request(Method::GET, &format!("/api/v1/requests/{id}"), ADMIN, None)
        .await;
    assert_eq!(
        after["request"]["request_digest"],
        created["request"]["request_digest"]
    );
    assert_eq!(after["request"]["signing"], created["request"]["signing"]);
    let receipt: nod_proto::DecisionSignatureRecord =
        serde_json::from_value(after["request"]["decisions"][0]["decision"]["signature"].clone())
            .unwrap();
    nod_proto::verify_payload(
        receipt.public_key.as_deref().unwrap(),
        receipt.signing_payload.as_bytes(),
        &receipt.signature,
    )
    .unwrap();
    let retained_key: String = sqlx::query_scalar(
        "SELECT signing_public_key FROM devices WHERE id = ? AND revoked_at IS NOT NULL",
    )
    .bind(&device_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(retained_key, signer.public_key().unwrap());
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/admin/users",
            ADMIN,
            Some(json!({"id":"owner","name":"Replacement"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // VACUUM INTO gives a consistent SQLite backup, including live WAL state.
    // No writers run while copying audit files in this fixture; operations
    // documentation requires stopping Nod for the same DB/audit boundary.
    let backup = tempfile::tempdir().unwrap();
    let backup_db = backup.path().join("nod.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(backup_db.to_str().unwrap())
        .execute(&app.pool)
        .await
        .unwrap();
    let backup_audit = backup.path().join("audit");
    tokio::fs::create_dir(&backup_audit).await.unwrap();
    let mut files = tokio::fs::read_dir(app.data_dir.join("audit"))
        .await
        .unwrap();
    let mut audit_has_receipt = false;
    while let Some(file) = files.next_entry().await.unwrap() {
        let content = tokio::fs::read_to_string(file.path()).await.unwrap();
        audit_has_receipt |= content.contains(&receipt.signature);
        tokio::fs::copy(file.path(), backup_audit.join(file.file_name()))
            .await
            .unwrap();
    }
    assert!(audit_has_receipt);
    let mut config = nod_server::Config::with_admin_token("admin-test-token");
    config.database_url = format!("sqlite://{}", backup_db.display());
    config.data_dir = backup.path().to_path_buf();
    let restored_state = nod_server::AppState::new(config).await.unwrap();
    let restored = nod_server::router(restored_state);
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let restored_response = restored
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/api/v1/requests/{id}"))
                .header("Authorization", "Bearer admin-test-token")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(restored_response.status(), StatusCode::OK);
    let restored: Value = serde_json::from_slice(
        &restored_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(restored["request"], after["request"]);
    let restored_receipt: nod_proto::DecisionSignatureRecord = serde_json::from_value(
        restored["request"]["decisions"][0]["decision"]["signature"].clone(),
    )
    .unwrap();
    nod_proto::verify_payload(
        restored_receipt.public_key.as_deref().unwrap(),
        restored_receipt.signing_payload.as_bytes(),
        &restored_receipt.signature,
    )
    .unwrap();
}
