use super::support::*;

#[tokio::test]
async fn device_can_set_notification_sound_preference() {
    let app = TestApp::new().await;
    let (_device_id, device_token) = app.enroll_device("Phone", "ios").await;

    let (status, updated) = app
        .request(
            Method::PUT,
            "/api/v1/devices/me/preferences",
            Some(&device_token),
            Some(json!({ "notification_sound": "nod_ping.wav" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, rejected) = app
        .request(
            Method::PUT,
            "/api/v1/devices/me/preferences",
            Some(&device_token),
            Some(json!({ "notification_sound": "../bad.wav" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
}

#[tokio::test]
async fn windows_and_linux_platforms_round_trip() {
    let app = TestApp::new().await;
    let (_windows_id, _windows_token) = app.enroll_device("Surface", "windows").await;
    let (_linux_id, _linux_token) = app.enroll_device("Workstation", "linux").await;

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/admin/devices",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    let platforms: Vec<_> = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|device| device["platform"].as_str().unwrap())
        .collect();
    assert!(platforms.contains(&"windows"), "{devices}");
    assert!(platforms.contains(&"linux"), "{devices}");
}

#[tokio::test]
async fn enroll_endpoint_returns_account_shape() {
    let app = TestApp::new().await;

    let (status, enrollment) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            Some("admin-test-token"),
            Some(json!({
                "expires_in_seconds": 600
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrollment}");
    let code = enrollment["code"].as_str().unwrap();
    let signing_key = test_signing_key_json();
    let key_id = signing_key["key_id"].clone();
    let public_key = signing_key["public_key"].clone();

    let (status, enrolled) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": code,
                "device_name": "Surface",
                "platform": "windows",
                "signing_key": signing_key
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
    assert_eq!(enrolled["user_id"], "owner");
    assert_eq!(enrolled["user_name"], "Owner");
    assert_eq!(enrolled["devices"][0]["platform"], "windows");
    assert_eq!(enrolled["devices"][0]["is_current"], true);
    assert_eq!(enrolled["devices"][0]["has_signing_key"], true);
    assert_eq!(enrolled["devices"][0]["signing_key_id"], key_id);
    assert_eq!(enrolled["devices"][0]["signing_public_key"], public_key);
    assert!(
        enrolled["devices"][0]["attestation"].is_null(),
        "{enrolled}"
    );
    assert!(enrolled["token"]
        .as_str()
        .unwrap()
        .starts_with("nod_device"));
}

#[tokio::test]
async fn enrollment_without_signing_key_is_rejected_without_consuming_code() {
    let app = TestApp::new().await;
    let (status, enrollment) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            Some("admin-test-token"),
            Some(json!({ "expires_in_seconds": 600 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrollment}");
    let code = enrollment["code"].as_str().unwrap();

    let (status, rejected) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": code,
                "device_name": "Keyless",
                "platform": "linux"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert!(
        rejected["message"]
            .as_str()
            .unwrap()
            .contains("signing_key"),
        "{rejected}"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);

    let (status, enrolled) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": code,
                "device_name": "Keyed",
                "platform": "linux",
                "signing_key": test_signing_key_json()
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
}

#[tokio::test]
async fn legacy_keyless_device_cannot_decide_without_a_key() {
    let app = TestApp::new().await;
    let (device_id, device_token) = app.enroll_device("Legacy", "linux").await;
    sqlx::query(
        "UPDATE devices SET signing_key_id=NULL, signing_key_algorithm=NULL, signing_public_key=NULL WHERE id=?",
    )
    .bind(&device_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let issuer_token = app.issuer_token().await;
    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(&issuer_token),
            Some(json!({
                "channel_id": "default",
                "title": "Legacy device",
                "options": [{ "id": "approve", "label": "Approve", "kind": "approve" }]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let request_id = created["request_id"].as_str().unwrap();

    let (status, rejected) = app
        .request(
            Method::POST,
            &format!("/api/v1/requests/{request_id}/options/approve"),
            Some(&device_token),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    let (status, view) = app
        .request(
            Method::GET,
            &format!("/api/v1/requests/{request_id}"),
            Some(&issuer_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["request"]["status"], "pending");
}

#[tokio::test]
async fn keyed_device_cannot_decide_without_a_signature() {
    let app = TestApp::new().await;
    let (_device_id, device_token) = app.enroll_device("Phone", "ios").await;
    let issuer_token = app.issuer_token().await;
    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(&issuer_token),
            Some(json!({ "channel_id": "default", "title": "Unsigned" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let request_id = created["request_id"].as_str().unwrap();
    let (status, rejected) = app
        .request(
            Method::POST,
            &format!("/api/v1/requests/{request_id}/options/dismiss"),
            Some(&device_token),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
}

#[tokio::test]
async fn push_registration_requires_native_app_id() {
    let app = TestApp::new().await;

    let (status, enrollment) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            Some("admin-test-token"),
            Some(json!({ "expires_in_seconds": 600 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrollment}");

    let (status, rejected) = app
        .request(
            Method::POST,
            "/api/v1/enroll",
            None,
            Some(json!({
                "code": enrollment["code"],
                "device_name": "iPhone",
                "platform": "ios",
                "push_provider": "apple_apns",
                "push_token": "provider-token",
                "signing_key": test_signing_key_json()
            })),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert!(
        rejected["message"]
            .as_str()
            .unwrap()
            .contains("native app id"),
        "{rejected}"
    );
}

#[tokio::test]
async fn malformed_attestation_is_recorded_without_blocking_enrollment() {
    let app = TestApp::new_with_config(|config| {
        config.device_attestation.apple_app_attest.team_id = Some("Y734633UDM".to_string());
        config.device_attestation.apple_app_attest.bundle_ids =
            vec!["com.batteryshark.Boop".to_string()];
    })
    .await;

    let (status, enrollment) = app
        .request(
            Method::POST,
            "/api/v1/admin/users/owner/enrollment-codes",
            Some("admin-test-token"),
            Some(json!({ "expires_in_seconds": 600 })),
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
                "platform": "ios",
                "signing_key": test_signing_key_json(),
                "attestation": {
                    "provider": "apple_app_attest",
                    "key_id": "test-key-id",
                    "attestation_object": "not-cbor"
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
    assert_eq!(
        enrolled["devices"][0]["attestation"]["status"], "failed",
        "{enrolled}"
    );
    assert_eq!(
        enrolled["devices"][0]["attestation"]["provider"], "apple_app_attest",
        "{enrolled}"
    );
    assert!(
        enrolled["devices"][0]["attestation"]["failure_reason"]
            .as_str()
            .unwrap()
            .contains("attestation object"),
        "{enrolled}"
    );
    assert!(!enrolled.to_string().contains("not-cbor"), "{enrolled}");

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/admin/devices",
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    assert_eq!(devices["devices"][0]["attestation"]["status"], "failed");
    assert!(!devices.to_string().contains("not-cbor"), "{devices}");
}

#[tokio::test]
async fn paired_device_can_list_rename_and_revoke_devices() {
    let app = TestApp::new().await;
    let (device_id, device_token) = app.enroll_device("Surface", "windows").await;

    let (status, me) = app
        .request(Method::GET, "/api/v1/users/me", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["user"]["id"], "owner");
    assert_eq!(me["current_device"]["id"], device_id);

    let (status, renamed) = app
        .request(
            Method::PUT,
            &format!("/api/v1/users/me/devices/{device_id}"),
            Some(&device_token),
            Some(json!({ "name": "Surface Studio" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["device"]["name"], "Surface Studio");

    let (status, devices) = app
        .request(
            Method::GET,
            "/api/v1/users/me/devices",
            Some(&device_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    assert_eq!(devices["devices"][0]["name"], "Surface Studio");

    let (status, revoked) = app
        .request(
            Method::DELETE,
            &format!("/api/v1/users/me/devices/{device_id}"),
            Some(&device_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");

    let (status, rejected) = app
        .request(Method::GET, "/api/v1/users/me", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
}
