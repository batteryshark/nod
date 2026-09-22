use super::support::*;

#[tokio::test]
async fn untargeted_request_goes_to_all_subscribed_users() {
    let app = TestApp::new().await;
    app.create_user("paul", "Paul").await;
    app.create_user("maya", "Maya").await;
    let (_paul_device_id, paul_token) = app
        .enroll_device_for_user("Paul's Phone", "ios", Some("paul"))
        .await;
    let (_maya_device_id, maya_token) = app
        .enroll_device_for_user("Maya's Phone", "ios", Some("maya"))
        .await;
    let issuer_token = app.issuer_token().await;

    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(&issuer_token),
            Some(json!({
                "channel_id": "default",
                "title": "Everyone",
                "summary": "No explicit recipients"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(created["request"]["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .any(|user_id| user_id == "paul"));
    assert!(created["request"]["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .any(|user_id| user_id == "maya"));

    let (status, paul_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&paul_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{paul_requests}");
    assert_eq!(paul_requests["requests"].as_array().unwrap().len(), 1);

    let (status, maya_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&maya_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{maya_requests}");
    assert_eq!(maya_requests["requests"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn targeted_request_is_visible_only_to_recipient_users() {
    let app = TestApp::new().await;
    app.create_user("paul", "Paul").await;
    app.create_user("maya", "Maya").await;
    let (_paul_device_id, paul_token) = app
        .enroll_device_for_user("Paul's Phone", "ios", Some("paul"))
        .await;
    let (_maya_device_id, maya_token) = app
        .enroll_device_for_user("Maya's Phone", "ios", Some("maya"))
        .await;
    let issuer_token = app.issuer_token().await;

    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(&issuer_token),
            Some(json!({
                "channel_id": "default",
                "recipients": ["paul"],
                "title": "Paul only",
                "summary": "Targeted notification"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["request"]["recipients"], json!(["paul"]));
    let request_id = created["request_id"].as_str().unwrap();

    let (status, paul_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&paul_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{paul_requests}");
    assert_eq!(paul_requests["requests"].as_array().unwrap().len(), 1);

    let (status, maya_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&maya_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{maya_requests}");
    assert_eq!(maya_requests["requests"].as_array().unwrap().len(), 0);

    let (status, forbidden) = app
        .request(
            Method::GET,
            &format!("/api/v1/requests/{request_id}"),
            Some(&maya_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
}

#[tokio::test]
async fn clearing_channel_is_per_user() {
    let app = TestApp::new().await;
    app.create_user("paul", "Paul").await;
    app.create_user("maya", "Maya").await;
    let (_mac_id, mac_token) = app
        .enroll_device_for_user("Paul's MacBook", "macos", Some("paul"))
        .await;
    let (_phone_id, phone_token) = app
        .enroll_device_for_user("Maya's Phone", "ios", Some("maya"))
        .await;
    let issuer_token = app.issuer_token().await;

    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/requests",
            Some(&issuer_token),
            Some(json!({
                "channel_id": "default",
                "title": "Visible",
                "summary": "Only cleared on one device"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");

    let request_id = created["request_id"].as_str().unwrap();
    let (status, cancelled) = app
        .request(
            Method::POST,
            &format!("/api/v1/requests/{request_id}/cancel"),
            Some("admin-test-token"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");

    let (status, cleared) = app
        .request(
            Method::POST,
            "/api/v1/devices/me/channels/default/clear",
            Some(&mac_token),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");

    let (status, mac_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&mac_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{mac_requests}");
    assert_eq!(mac_requests["requests"].as_array().unwrap().len(), 0);

    let (status, phone_requests) = app
        .request(Method::GET, "/api/v1/requests", Some(&phone_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{phone_requests}");
    assert_eq!(phone_requests["requests"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn channel_list_includes_device_subscription_state() {
    let app = TestApp::new().await;
    let (_device_id, device_token) = app.enroll_device("Phone", "ios").await;

    let (status, initial) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    assert_eq!(initial["channels"][0]["id"], "default");
    assert_eq!(initial["channels"][0]["subscribed"], true);

    let (status, updated) = app
        .request(
            Method::PUT,
            "/api/v1/devices/me/subscriptions/default",
            Some(&device_token),
            Some(json!({ "subscribed": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, channels) = app
        .request(Method::GET, "/api/v1/channels", Some(&device_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    assert_eq!(channels["channels"][0]["subscribed"], false);
}
