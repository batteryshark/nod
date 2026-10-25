#[path = "api/support.rs"]
// This integration target uses only part of the shared API fixture helpers.
#[allow(dead_code, unused_imports)]
mod support;
use support::*;

const ADMIN: Option<&str> = Some("admin-test-token");

#[tokio::test]
async fn signed_nonce_rolls_back_with_journal_and_exact_retry_succeeds() {
    let app = TestApp::new().await;
    let (device, token) = app.enroll_device("Signed", "ios").await;
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    sqlx::query("UPDATE devices SET signing_key_id='key',signing_key_algorithm='p256_ecdsa_sha256',signing_public_key=? WHERE id=?")
        .bind(URL_SAFE_NO_PAD.encode(key.public_key().as_ref())).bind(&device).execute(&app.pool).await.unwrap();
    let created = create(
        &app,
        json!({"title":"signed","options":[{"id":"approve","kind":"approve","label":"Approve"}]}),
    )
    .await;
    let id = created["request_id"].as_str().unwrap();
    let digest = created["request"]["request_digest"].as_str().unwrap();
    let payload = decision_payload(DecisionPayload {
        request_id: id,
        request_digest: digest,
        option_id: "approve",
        option_kind: "approve",
        user_id: "owner",
        device_id: &device,
        key_id: "key",
        nonce: "rollback",
        signed_at: "2026-05-31T12:00:00.000Z",
        text: "ship",
    });
    let body = json!({"text":"ship","signature":{"key_id":"key","algorithm":"p256_ecdsa_sha256","nonce":"rollback","signed_at":"2026-05-31T12:00:00.000Z","request_digest":digest,"signature":URL_SAFE_NO_PAD.encode(key.sign(&rng,payload.as_bytes()).unwrap())}});
    let uri = format!("/api/v1/requests/{id}/options/approve");
    sqlx::raw_sql("CREATE TRIGGER fail_signed BEFORE INSERT ON lifecycle_events WHEN NEW.revision>1 BEGIN SELECT RAISE(ABORT,'injected'); END;").execute(&app.pool).await.unwrap();
    assert_eq!(
        app.request(Method::POST, &uri, Some(&token), Some(body.clone()))
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM decision_nonces")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_signed")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.request(Method::POST, &uri, Some(&token), Some(body.clone()))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        app.request(Method::POST, &uri, Some(&token), Some(body))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM decision_nonces")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn shared_answers_and_cancel_race_only_one_terminal_commit() {
    let app = TestApp::new().await;
    let (_, a) = app.enroll_device("A", "ios").await;
    let (_, b) = app.enroll_device("B", "ios").await;
    let created = create(&app, json!({"title":"race answers"})).await;
    let id = created["request_id"].as_str().unwrap();
    let cancel = format!("/api/v1/requests/{id}/cancel");
    let (a, b, c) = tokio::join!(
        app.decide(&a, id, "dismiss", json!({})),
        app.decide(&b, id, "dismiss", json!({})),
        app.request(Method::POST, &cancel, ADMIN, None)
    );
    let statuses = [a.0, b.0, c.0];
    assert_eq!(statuses.iter().filter(|&&s| s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::CONFLICT)
            .count(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lifecycle_events")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn scoped_replay_advances_over_hidden_rows_and_rechecks_revocation() {
    let app = TestApp::new().await;
    create(&app, json!({"title":"hidden"})).await;
    let scoped = app
        .issuer_token_with_scopes(["requests:read:elsewhere"])
        .await;
    let (status, page) = app
        .request(
            Method::GET,
            "/api/v1/integrations/events?limit=1",
            Some(&scoped),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["events"], json!([]));
    assert_eq!(page["next_cursor"], 1);
    assert_eq!(page["high_water"], 1);
    let reader = app
        .issuer_token_with_scopes(["requests:read:default"])
        .await;
    let (_, page) = app
        .request(
            Method::GET,
            "/api/v1/integrations/events",
            Some(&reader),
            None,
        )
        .await;
    let id = page["events"][0]["envelope"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/integrations/events/{id}");
    assert_eq!(
        app.request(Method::GET, &uri, Some(&scoped), None).await.0,
        StatusCode::FORBIDDEN
    );
    let no_read = app.issuer_token_with_scopes(["requests:cancel"]).await;
    assert_eq!(
        app.request(
            Method::GET,
            "/api/v1/integrations/events",
            Some(&no_read),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE issuer_tokens SET revoked_at='2026-10-23T00:00:00.000Z'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.request(Method::GET, &uri, Some(&reader), None).await.0,
        StatusCode::UNAUTHORIZED
    );
}

async fn create(app: &TestApp, value: Value) -> Value {
    let (status, body) = app
        .request(Method::POST, "/api/v1/requests", ADMIN, Some(value))
        .await;
    assert!(status.is_success(), "{status}: {body}");
    body
}

#[tokio::test]
async fn callback_insert_failure_rolls_back_answer_and_revision() {
    let app = TestApp::new().await;
    let (_, token) = app.enroll_device("Phone", "ios").await;
    let created = create(
        &app,
        json!({"title":"answer rollback","callback_url":"http://127.0.0.1:9"}),
    )
    .await;
    let id = created["request_id"].as_str().unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_callback BEFORE INSERT ON callback_deliveries BEGIN SELECT RAISE(ABORT,'injected'); END;").execute(&app.pool).await.unwrap();
    assert_eq!(
        app.decide(&token, id, "dismiss", json!({})).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let state: (String, i64) = sqlx::query_as("SELECT status,revision FROM requests WHERE id=?")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(state, ("pending".into(), 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lifecycle_events")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("DROP TRIGGER fail_callback")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.decide(&token, id, "dismiss", json!({})).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn journal_failure_rolls_back_create_and_dedupe_obligations() {
    let app = TestApp::new().await;
    sqlx::raw_sql("CREATE TRIGGER fail_journal BEFORE INSERT ON lifecycle_events BEGIN SELECT RAISE(ABORT,'injected'); END;").execute(&app.pool).await.unwrap();
    let body = json!({"title":"rollback","idempotency_key":"key","dedupe_key":"pending"});
    let (status, _) = app
        .request(Method::POST, "/api/v1/requests", ADMIN, Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    for table in [
        "requests",
        "request_options",
        "request_recipients",
        "request_idempotency",
        "push_deliveries",
        "lifecycle_events",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&app.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    sqlx::query("DROP TRIGGER fail_journal")
        .execute(&app.pool)
        .await
        .unwrap();
    create(&app, body.clone()).await;
    create(&app, body).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lifecycle_events")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn origin_binding_dedupe_and_pending_migration_are_immutable() {
    let app = TestApp::new().await;
    let origin = json!({"session":"ses_one","directory":"/tower","reference":"opaque"});
    let value = json!({"title":"origin","dedupe_key":"one","origin":origin});
    let created = create(&app, value.clone()).await;
    let id = created["request_id"].as_str().unwrap();
    create(&app, value).await;
    let (status,_) = app.request(Method::POST,"/api/v1/requests",ADMIN,Some(json!({"title":"origin","dedupe_key":"one","origin":{"session":"ses_two","directory":"/tower"}}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let uri = format!("/api/v1/integrations/requests/{id}/origin");
    assert_eq!(
        app.request(Method::POST, &uri, ADMIN, Some(origin)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        app.request(
            Method::POST,
            &uri,
            ADMIN,
            Some(json!({"session":"ses_two","directory":"/tower"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let legacy = create(&app, json!({"title":"legacy","idempotency_key":"old"})).await;
    let legacy_id = legacy["request_id"].as_str().unwrap();
    assert_eq!(
        app.request(
            Method::POST,
            &format!("/api/v1/integrations/requests/{legacy_id}/origin"),
            ADMIN,
            Some(json!({"session":"ses_old","directory":"/tower"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let stored: String =
        sqlx::query_scalar("SELECT private_detail_json FROM lifecycle_events WHERE request_id=?")
            .bind(legacy_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(serde_json::from_str::<Value>(&stored).unwrap()["origin"].is_null());
}

#[tokio::test]
async fn individual_answers_and_frozen_callbacks_survive_pruning() {
    let app = TestApp::new().await;
    app.create_user("second", "Second").await;
    let (_, a) = app.enroll_device("A", "ios").await;
    let (_, b) = app.enroll_device_for_user("B", "ios", Some("second")).await;
    let created = create(&app,json!({"title":"private","recipients":["owner","second"],"decision_resolution":"per_user","callback_url":"http://127.0.0.1:9/callback"})).await;
    let id = created["request_id"].as_str().unwrap();
    for (token, text) in [(&a, "first"), (&b, "second")] {
        let (status, body) = app.decide(token, id, "dismiss", json!({"text":text})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        app.decide(&a, id, "dismiss", json!({})).await.0,
        StatusCode::CONFLICT
    );
    let details: Vec<String> = sqlx::query_scalar("SELECT private_detail_json FROM lifecycle_events WHERE kind='nod.answer.recorded' ORDER BY revision").fetch_all(&app.pool).await.unwrap();
    assert_eq!(details.len(), 2);
    for (detail, text) in details.iter().zip(["first", "second"]) {
        assert_eq!(
            serde_json::from_str::<Value>(detail).unwrap()["decision"]["text"],
            text
        );
    }
    let callbacks: Vec<String> = sqlx::query_scalar(
        "SELECT payload_json FROM callback_deliveries ORDER BY request_revision",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&callbacks[0]).unwrap()["status"],
        "pending"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&callbacks[1]).unwrap()["status"],
        "resolved"
    );
    sqlx::query("DELETE FROM requests WHERE id=?")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM callback_deliveries")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        2
    );
    let (_, replay) = app
        .request(Method::GET, "/api/v1/integrations/events", ADMIN, None)
        .await;
    assert_eq!(replay["events"].as_array().unwrap().len(), 3);
    let event_id = replay["events"][1]["envelope"]["id"].as_str().unwrap();
    let detail_uri = format!("/api/v1/integrations/events/{event_id}");
    assert_eq!(
        app.request(Method::GET, &detail_uri, Some(&a), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.request(Method::GET, &detail_uri, ADMIN, None).await.1["decision"]["text"],
        "first"
    );
}

#[tokio::test]
async fn competing_cancel_commits_one_terminal_event_and_insertion_failure_rolls_back() {
    let app = TestApp::new().await;
    let created = create(&app, json!({"title":"race"})).await;
    let id = created["request_id"].as_str().unwrap();
    let uri = format!("/api/v1/requests/{id}/cancel");
    sqlx::raw_sql("CREATE TRIGGER fail_terminal BEFORE INSERT ON lifecycle_events WHEN NEW.revision>1 BEGIN SELECT RAISE(ABORT,'injected'); END;").execute(&app.pool).await.unwrap();
    assert_eq!(
        app.request(Method::POST, &uri, ADMIN, None).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let state: (String, i64) = sqlx::query_as("SELECT status,revision FROM requests WHERE id=?")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(state, ("pending".into(), 1));
    sqlx::query("DROP TRIGGER fail_terminal")
        .execute(&app.pool)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        app.request(Method::POST, &uri, ADMIN, None),
        app.request(Method::POST, &uri, ADMIN, None)
    );
    assert!(
        a.0 == StatusCode::OK && b.0 == StatusCode::CONFLICT
            || b.0 == StatusCode::OK && a.0 == StatusCode::CONFLICT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lifecycle_events")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        2
    );
}
