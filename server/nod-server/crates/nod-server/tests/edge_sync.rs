// Real local WebSocket, with direct DB mutation deliberately bypassing all fanout.
#[path = "api/support.rs"]
#[allow(dead_code, unused_imports)]
mod support;

use futures_util::StreamExt;
use support::TestApp;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn quiet_socket_resyncs_and_revocation_without_broadcast_closes_it() {
    let app = TestApp::new().await;
    let (device_id, token) = app.enroll_device("Quiet", "linux").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, _) = connect_async(format!("ws://{address}/api/v1/sync?token={token}"))
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
