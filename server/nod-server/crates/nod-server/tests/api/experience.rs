use super::support::*;

const ADMIN: Option<&str> = Some("admin-test-token");

async fn create(app: &TestApp, body: Value) -> String {
    let (status, created) = app
        .request(Method::POST, "/api/v1/requests", ADMIN, Some(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    created["request_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn history_paging_search_and_clear_preserve_pending_requests() {
    let app = TestApp::new().await;
    let (_, token) = app.enroll_device("History", "linux").await;
    let pending = create(&app, json!({"title":"Find ME pending"})).await;
    let mut history = Vec::new();
    for index in 0..5 {
        let id = create(&app, json!({"title":format!("Find me handled {index}")})).await;
        app.request(
            Method::POST,
            &format!("/api/v1/requests/{id}/cancel"),
            ADMIN,
            None,
        )
        .await;
        history.push(id);
    }
    let (_, first) = app
        .request(
            Method::GET,
            "/api/v1/requests?search=FIND%20ME&limit=2",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(first["requests"].as_array().unwrap().len(), 3, "{first}");
    assert_eq!(first["requests"][0]["id"], pending);
    let mut seen = first["requests"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut cursor = first["next_cursor"].as_str().map(str::to_string);
    while let Some(before) = cursor {
        let (status, next) = app
            .request(
                Method::GET,
                &format!("/api/v1/requests?search=find%20me&limit=2&before={before}"),
                Some(&token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{next}");
        seen.extend(
            next["requests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string()),
        );
        cursor = next["next_cursor"].as_str().map(str::to_string);
    }
    seen.sort();
    history.sort();
    assert_eq!(seen, history);
    let (_, empty) = app
        .request(
            Method::GET,
            "/api/v1/requests?search=missing",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(empty["requests"], json!([]));
    app.request(
        Method::POST,
        "/api/v1/devices/me/channels/default/clear",
        Some(&token),
        Some(json!({})),
    )
    .await;
    let (_, visible) = app
        .request(Method::GET, "/api/v1/requests", Some(&token), None)
        .await;
    assert_eq!(visible["requests"].as_array().unwrap().len(), 1);
    assert_eq!(visible["requests"][0]["id"], pending);
    let (_, all) = app
        .request(
            Method::GET,
            "/api/v1/requests?include_cleared=true",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(all["requests"].as_array().unwrap().len(), 6);
    let (status, _) = app
        .request(
            Method::GET,
            "/api/v1/requests?before=invalid",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn explicit_targeting_overrides_unsubscribe_without_revealing_broadcasts() {
    let app = TestApp::new().await;
    let (_, token) = app.enroll_device("Target", "linux").await;
    create(&app, json!({"title":"Earlier broadcast"})).await;
    app.request(
        Method::PUT,
        "/api/v1/devices/me/subscriptions/default",
        Some(&token),
        Some(json!({"subscribed":false})),
    )
    .await;
    let explicit = create(&app, json!({"title":"Explicit", "recipients":["owner"]})).await;
    let (_, visible) = app
        .request(Method::GET, "/api/v1/requests", Some(&token), None)
        .await;
    assert_eq!(
        visible["requests"].as_array().unwrap().len(),
        1,
        "{visible}"
    );
    assert_eq!(visible["requests"][0]["id"], explicit);
}

#[tokio::test]
async fn old_pending_requests_remain_visible_outside_terminal_retention() {
    let app = TestApp::new_with_config(|config| config.retention_days = 1).await;
    let (_, token) = app.enroll_device("Pending", "linux").await;
    let pending = create(&app, json!({"title":"Still waiting"})).await;
    sqlx::query("UPDATE requests SET created_at='2000-01-01T00:00:00.000Z',updated_at='2000-01-01T00:00:00.000Z' WHERE id=?").bind(&pending).execute(&app.pool).await.unwrap();
    let (_, visible) = app
        .request(Method::GET, "/api/v1/requests", Some(&token), None)
        .await;
    assert_eq!(visible["requests"][0]["id"], pending);
}

#[tokio::test]
async fn idempotency_survives_resolution_and_rejects_changed_content() {
    let app = TestApp::new().await;
    let body = json!({"title":"Safely retry", "idempotency_key":"operation-1"});
    let first = create(&app, body.clone()).await;
    app.request(
        Method::POST,
        &format!("/api/v1/requests/{first}/cancel"),
        ADMIN,
        None,
    )
    .await;
    let retry = create(&app, body).await;
    assert_eq!(first, retry);
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            ADMIN,
            Some(json!({"title":"Changed", "idempotency_key":"operation-1"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM requests")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn callbacks_reject_disallowed_origins_and_credentials() {
    let app = TestApp::new_with_config(|config| {
        config.callback_allowed_origins = Some(vec!["https://allowed.example".to_string()])
    })
    .await;
    for url in [
        "https://other.example/callback",
        "https://allowed.example.evil.test/callback",
        "http://allowed.example/callback",
        "https://user:secret@allowed.example/callback",
    ] {
        let (status, _) = app
            .request(
                Method::POST,
                "/api/v1/requests",
                ADMIN,
                Some(json!({"title":"Callback", "callback_url":url})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}");
    }
    create(
        &app,
        json!({"title":"Allowed", "callback_url":"https://allowed.example/callback?key=secret"}),
    )
    .await;
}

#[tokio::test]
async fn callback_latency_is_not_part_of_decision_acknowledgement() {
    use axum::{routing::post, Router};
    use std::time::Duration;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, received) = tokio::sync::oneshot::channel();
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Some(seen)));
    let route = Router::new().route(
        "/slow",
        post(move || {
            let seen = seen.clone();
            async move {
                if let Some(seen) = seen.lock().await.take() {
                    let _ = seen.send(());
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
                StatusCode::OK
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, route).await.unwrap() });
    let app = TestApp::new().await;
    let (_, token) = app.enroll_device("Decide", "linux").await;
    let id = create(
        &app,
        json!({"title":"Slow callback", "callback_url":format!("http://{address}/slow")}),
    )
    .await;
    let (status, body) = tokio::time::timeout(
        Duration::from_millis(500),
        app.request(
            Method::POST,
            &format!("/api/v1/requests/{id}/options/dismiss"),
            Some(&token),
            Some(json!({})),
        ),
    )
    .await
    .expect("decision must not wait for callback");
    assert_eq!(status, StatusCode::OK, "{body}");
    tokio::time::timeout(Duration::from_secs(1), received)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}

#[tokio::test]
async fn activity_is_admin_only_and_reports_durable_push_jobs() {
    let app = TestApp::new().await;
    let (id, token) = app.enroll_device("Push", "ios").await;
    app.request(
        Method::PUT,
        "/api/v1/devices/me/push-token",
        Some(&token),
        Some(json!({"provider":"apple_apns","token":"token","native_app_id":"test.bundle"})),
    )
    .await;
    let request = create(&app, json!({"title":"Trace push"})).await;
    let (status, activity) = app
        .request(Method::GET, "/api/v1/admin/activity", ADMIN, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{activity}");
    assert_eq!(activity["deliveries"][0]["request_id"], request);
    assert_eq!(activity["deliveries"][0]["device_id"], id);
    assert_eq!(activity["audit"]["healthy"], true);
    let (status, _) = app
        .request(Method::GET, "/api/v1/admin/activity", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn readme_json_example_matches_the_issuer_contract() {
    let readme = include_str!("../../../../../../README.md");
    let example = readme
        .split("```json")
        .nth(1)
        .expect("README JSON example")
        .split("```")
        .next()
        .unwrap();
    serde_json::from_str::<nod_proto::CreateDecisionRequest>(example)
        .expect("README example must be a valid create request");
}

#[tokio::test]
async fn notification_preferences_are_per_device_and_never_hide_inbox_content() {
    let app = TestApp::new().await;
    let (_, first) = app.enroll_device("Quiet", "ios").await;
    let (_, second) = app.enroll_device("Normal", "ios").await;
    let snooze = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (status,result)=app.request(Method::PUT,"/api/v1/devices/me/notification-preferences",Some(&first),Some(json!({"hide_content":true,"muted_channels":["default","default"],"snoozed_until":snooze}))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (_, me) = app
        .request(Method::GET, "/api/v1/users/me", Some(&first), None)
        .await;
    assert_eq!(
        me["current_device"]["notification_preferences"]["hide_content"],
        true
    );
    assert_eq!(
        me["current_device"]["notification_preferences"]["muted_channels"],
        json!(["default"])
    );
    let (_, other) = app
        .request(Method::GET, "/api/v1/users/me", Some(&second), None)
        .await;
    assert_eq!(
        other["current_device"]["notification_preferences"]["hide_content"],
        false
    );
    let request = create(&app, json!({"title":"Private details remain readable"})).await;
    let (_, inbox) = app
        .request(Method::GET, "/api/v1/requests", Some(&first), None)
        .await;
    assert_eq!(inbox["requests"][0]["id"], request);
    assert_eq!(
        inbox["requests"][0]["title"],
        "Private details remain readable"
    );
    let (status, _) = app
        .request(
            Method::PUT,
            "/api/v1/devices/me/notification-preferences",
            Some(&first),
            Some(json!({"muted_channels":["bad channel"]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
