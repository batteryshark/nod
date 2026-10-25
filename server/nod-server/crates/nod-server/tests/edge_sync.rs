// Real local WebSocket, with direct DB mutation deliberately bypassing all fanout.
#[path = "api/support.rs"]
#[allow(dead_code, unused_imports)]
mod support;

use futures_util::StreamExt;
use support::{sync_upgrade_request, TestApp};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn sync_socket_accepts_only_the_authorization_header() {
    let app = TestApp::new().await;
    let (_device_id, token) = app.enroll_device("Header", "linux").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // The former query-string form is gone, not merely deprecated.
    let rejected = connect_async(format!("ws://{address}/api/v1/sync?token={token}")).await;
    assert!(
        matches!(
            rejected,
            Err(tokio_tungstenite::tungstenite::Error::Http(ref response))
                if response.status() == 401
        ),
        "{rejected:?}"
    );
    let rejected = connect_async(format!("ws://{address}/api/v1/sync")).await;
    assert!(
        matches!(
            rejected,
            Err(tokio_tungstenite::tungstenite::Error::Http(ref response))
                if response.status() == 401
        ),
        "{rejected:?}"
    );
    let rejected = connect_async(sync_upgrade_request(
        &format!("ws://{address}/api/v1/sync"),
        "nod_device_not-a-real-token",
    ))
    .await;
    assert!(
        matches!(
            rejected,
            Err(tokio_tungstenite::tungstenite::Error::Http(ref response))
                if response.status() == 401
        ),
        "{rejected:?}"
    );

    let (mut socket, _) = connect_async(sync_upgrade_request(
        &format!("ws://{address}/api/v1/sync"),
        &token,
    ))
    .await
    .unwrap();
    let hello = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&hello).unwrap()["kind"],
        "hello"
    );
    server.abort();
}

#[tokio::test]
async fn quiet_socket_resyncs_and_revocation_without_broadcast_closes_it() {
    let app = TestApp::new().await;
    let (device_id, token) = app.enroll_device("Quiet", "linux").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, _) = connect_async(sync_upgrade_request(
        &format!("ws://{address}/api/v1/sync"),
        &token,
    ))
    .await
    .unwrap();
    let hello = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&hello).unwrap()["kind"],
        "hello"
    );

    // Requiring two resyncs proves the safety net is periodic, not merely an
    // upgrade message. The initial authorization is checked before hello.
    for _ in 0..2 {
        let message = timeout(Duration::from_secs(18), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(envelope["kind"], "resync_required");
        assert_eq!(envelope["payload"], serde_json::json!({}));
    }
    sqlx::query("UPDATE devices SET revoked_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&device_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let message = timeout(Duration::from_secs(18), socket.next())
        .await
        .unwrap();
    assert!(matches!(message, None | Some(Ok(Message::Close(_)))));
    server.abort();
}
