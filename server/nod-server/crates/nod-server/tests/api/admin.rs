use super::support::*;

#[tokio::test]
async fn admin_page_serves_static_asset() {
    let app = TestApp::new().await;

    let (status, headers, body) = app.request_text(Method::GET, "/admin").await;

    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html")));
    assert!(body.contains("<title>Nod Admin</title>"));
}

#[tokio::test]
async fn admin_session_login_logout_and_cookie_auth() {
    let app = TestApp::new().await;

    let (status, rejected) = app
        .request(Method::GET, "/api/v1/admin/summary", None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");

    let (status, forbidden) = app
        .request(
            Method::POST,
            "/admin/session",
            None,
            Some(json!({ "token": "wrong" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let (status, headers, logged_in) = app
        .request_raw(
            Method::POST,
            "/admin/session",
            None,
            None,
            Some(json!({ "token": "admin-test-token" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{logged_in}");
    let set_cookie = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set_cookie.contains("nod_admin_session="), "{set_cookie}");
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(set_cookie.contains("Max-Age=43200"), "{set_cookie}");
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let (status, _headers, summary) = app
        .request_with_cookie(Method::GET, "/api/v1/admin/summary", &cookie, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["channels"], 1);

    let (status, headers, logged_out) = app
        .request_with_cookie(Method::DELETE, "/admin/session", &cookie, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{logged_out}");
    let set_cookie = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");
}

#[tokio::test]
async fn admin_settings_redacts_secrets() {
    let app = TestApp::new_with_config(|config| {
        config.retention_days = 14;
        config.notifications.apns_relay.url = Some("https://relay.example.com".to_string());
        config.notifications.apns_relay.native_app_id = Some("com.example.NodTests".to_string());
        config.notifications.apns_relay.tls.client_cert_path =
            Some("tests/fixtures/relay-tls/client.crt".into());
        config.notifications.apns_relay.tls.client_key_path =
            Some("tests/fixtures/relay-tls/client.key".into());
        config.notifications.apns_relay.tls.ca_cert_path =
            Some("tests/fixtures/relay-tls/server-ca.crt".into());
        config.device_attestation.apple_app_attest.team_id = Some("TEAMID".to_string());
        config.device_attestation.apple_app_attest.bundle_ids = vec![
            "com.example.Nod".to_string(),
            "com.example.NodMac".to_string(),
        ];
    })
    .await;

    let (status, settings) = app
        .request(
            Method::GET,
            "/api/v1/admin/settings",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["notification_delivery_mode"], "push");
    assert_eq!(settings["push_route"], "apns_relay");
    assert_eq!(settings["retention_days"], 14);
    assert!(settings.get("apple_apns").is_none());
    assert_eq!(settings["apns_relay"]["client_enabled"], true);
    assert_eq!(settings["apns_relay"]["url"], "https://relay.example.com");
    assert_eq!(
        settings["apns_relay"]["native_app_id"],
        "com.example.NodTests"
    );
    assert_eq!(settings["apns_relay"]["client_cert_configured"], true);
    assert_eq!(settings["apns_relay"]["client_key_configured"], true);
    assert_eq!(settings["apns_relay"]["ca_cert_configured"], true);
    assert_eq!(
        settings["device_attestation"]["apple_app_attest"]["mode"],
        "report_only"
    );
    assert_eq!(
        settings["device_attestation"]["apple_app_attest"]["team_id_configured"],
        true
    );
    assert_eq!(
        settings["device_attestation"]["apple_app_attest"]["bundle_ids"],
        json!(["com.example.Nod", "com.example.NodMac"])
    );
    assert_eq!(
        settings["device_attestation"]["apple_app_attest"]["environment"],
        "production"
    );

    let text = settings.to_string();
    assert!(!text.contains("client.key"), "{text}");
}

#[tokio::test]
async fn device_responses_report_websocket_delivery_without_push_route() {
    let app = TestApp::new().await;
    let enrollment_body = json!({
        "expires_in_seconds": 600
    });
    let (status, enrollment) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            Some("admin-test-token"),
            Some(enrollment_body),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrollment}");
    let code = enrollment["code"].as_str().unwrap();

    let (status, enrolled) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": code,
                "device_name": "Phone",
                "platform": "ios"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
    assert_eq!(enrolled["notification_delivery"]["mode"], "websocket");

    let device_token = enrolled["token"].as_str().unwrap();
    let (status, me) = app
        .request(Method::GET, "/api/v1/users/me", Some(device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["notification_delivery"]["mode"], "websocket");
}

#[tokio::test]
async fn device_responses_hide_apns_relay_route_behind_push_delivery() {
    let app = TestApp::new_with_config(|config| {
        config.notifications.apns_relay.url = Some("https://relay.example.com".to_string());
        config.notifications.apns_relay.native_app_id = Some("com.example.NodTests".to_string());
        config.notifications.apns_relay.tls.client_cert_path =
            Some("tests/fixtures/relay-tls/client.crt".into());
        config.notifications.apns_relay.tls.client_key_path =
            Some("tests/fixtures/relay-tls/client.key".into());
        config.notifications.apns_relay.tls.ca_cert_path =
            Some("tests/fixtures/relay-tls/server-ca.crt".into());
    })
    .await;

    let (_device_id, device_token) = app.enroll_device("Phone", "ios").await;
    let (status, me) = app
        .request(Method::GET, "/api/v1/users/me", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["notification_delivery"]["mode"], "websocket");
    app.request(
        Method::PUT,
        "/api/v1/devices/me/push-token",
        Some(&device_token),
        Some(
            json!({"provider":"apple_apns","token":"device-token","native_app_id":"wrong.bundle"}),
        ),
    )
    .await;
    let (_, wrong_route) = app
        .request(Method::GET, "/api/v1/users/me", Some(&device_token), None)
        .await;
    assert_eq!(wrong_route["notification_delivery"]["mode"], "websocket");
    app.request(Method::PUT, "/api/v1/devices/me/push-token", Some(&device_token), Some(json!({"provider":"apple_apns","token":"device-token","native_app_id":"com.example.NodTests"}))).await;
    let (_, me) = app
        .request(Method::GET, "/api/v1/users/me", Some(&device_token), None)
        .await;
    assert_eq!(me["notification_delivery"]["mode"], "push");
    assert!(me["push_route"].is_null(), "{me}");
}

#[tokio::test]
async fn admin_lists_hide_secrets_and_token_revocation_blocks_use() {
    let app = TestApp::new().await;

    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/admin/issuer-tokens",
            Some("admin-test-token"),
            Some(json!({
                "name": "agent",
                "scopes": ["requests:write", "requests:read"]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let token_id = created["id"].as_str().unwrap();
    let token = created["token"].as_str().unwrap();

    let (status, listed) = app
        .request(
            Method::GET,
            "/api/v1/admin/issuer-tokens",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["tokens"][0]["id"], token_id);
    let listed_text = listed.to_string();
    assert!(!listed_text.contains(token), "{listed_text}");
    assert!(!listed_text.contains("token_hash"), "{listed_text}");
    assert!(listed["tokens"][0]["token"].is_null(), "{listed}");

    let (status, revoked) = app
        .request(
            Method::DELETE,
            &format!("/api/v1/admin/issuer-tokens/{token_id}"),
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");

    let (status, rejected) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(token),
            Some(json!({
                "channel_id": "default",
                "title": "Blocked after revoke",
                "summary": "This should fail"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
}

#[tokio::test]
async fn admin_can_revoke_device_and_manage_subscriptions() {
    let app = TestApp::new().await;
    let (device_id, device_token) = app.enroll_device("Phone", "ios").await;

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/admin/devices",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    assert_eq!(devices["devices"][0]["id"], device_id);
    assert_eq!(devices["devices"][0]["has_push_token"], false);
    assert!(devices["devices"][0]["push_token"].is_null(), "{devices}");

    let (status, updated) = app
        .request(
            Method::PUT,
            &format!("/api/v1/admin/devices/{device_id}/subscriptions/default"),
            Some("admin-test-token"),
            Some(json!({ "subscribed": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, channels) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    assert_eq!(channels["channels"][0]["subscribed"], false);

    let (status, revoked) = app
        .request(
            Method::DELETE,
            &format!("/api/v1/admin/devices/{device_id}"),
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");

    let (status, rejected) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
}

#[tokio::test]
async fn device_can_update_generic_push_token() {
    let app = TestApp::new().await;
    let (device_id, device_token) = app.enroll_device("Phone", "ios").await;

    let (status, updated) = app
        .request(
            Method::PUT,
            "/api/v1/devices/me/push-token",
            Some(&device_token),
            Some(json!({
                "provider": "apple_apns",
                "native_app_id": "com.example.NodTests",
                "token": "provider-token"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/admin/devices",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    assert_eq!(devices["devices"][0]["id"], device_id);
    assert_eq!(
        devices["devices"][0]["native_app_id"],
        "com.example.NodTests"
    );
    assert_eq!(devices["devices"][0]["push_provider"], "apple_apns");
    assert_eq!(devices["devices"][0]["has_push_token"], true);
    assert!(devices["devices"][0]["push_token"].is_null(), "{devices}");
}

#[tokio::test]
async fn admin_can_manage_subscriptions_on_users() {
    let app = TestApp::new().await;
    let (_device_id, device_token) = app.enroll_device("Phone", "ios").await;

    let (status, created_channel) = app
        .request(
            Method::POST,
            "/api/v1/admin/channels",
            Some("admin-test-token"),
            Some(json!({
                "id": "alerts",
                "name": "Alerts",
                "emoji": "🚨"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created_channel}");

    let (status, subscriptions) = app
        .request(
            Method::GET,
            "/api/v1/admin/users/owner/subscriptions",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{subscriptions}");
    assert_eq!(subscriptions["channels"].as_array().unwrap().len(), 2);
    assert_eq!(
        subscriptions["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|channel| channel["subscribed"] == true)
            .count(),
        1
    );
    assert_eq!(
        subscriptions["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["id"] == "alerts")
            .unwrap()["subscribed"],
        false
    );

    let (status, updated) = app
        .request(
            Method::PUT,
            "/api/v1/admin/users/owner/subscriptions",
            Some("admin-test-token"),
            Some(json!({
                "updates": [
                    { "channel_id": "default", "subscribed": false },
                    { "channel_id": "alerts", "subscribed": false }
                ]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        updated["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|channel| channel["subscribed"] == true)
            .count(),
        0
    );

    let (status, users) = app
        .request(
            Method::GET,
            "/api/v1/admin/users",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert_eq!(users["users"][0]["subscribed_channel_count"], 0);

    let (status, device_channels) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{device_channels}");
    assert_eq!(
        device_channels["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|channel| channel["subscribed"] == true)
            .count(),
        0
    );
}

#[tokio::test]
async fn admin_can_manage_users_and_enroll_devices_to_users() {
    let app = TestApp::new().await;

    let (status, initial) = app
        .request(
            Method::GET,
            "/api/v1/admin/users",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    assert!(initial["users"]
        .as_array()
        .unwrap()
        .iter()
        .any(|user| user["id"] == "owner"));

    app.create_user("paul", "Paul").await;
    let (device_id, device_token) = app
        .enroll_device_for_user("Paul's Phone", "ios", Some("paul"))
        .await;

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/admin/devices",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    let device = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["id"] == device_id)
        .unwrap();
    assert_eq!(device["user_id"], "paul");
    assert_eq!(device["user_name"], "Paul");

    let (status, channels) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    assert_eq!(channels["channels"][0]["subscribed"], true);
}
