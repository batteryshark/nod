//! At-least-once delivery of immutable, transactionally queued answer bodies.
use crate::{db::callback_deliveries, error::ApiError, state::AppState};
use std::time::Duration;
use tokio::{
    task::JoinSet,
    time::{sleep, timeout},
};

pub(crate) fn validate_destination(
    allowed: Option<&[String]>,
    callback: Option<&str>,
) -> Result<(), ApiError> {
    let Some(callback) = callback else {
        return Ok(());
    };
    let url = url::Url::parse(callback)
        .map_err(|_| ApiError::BadRequest("invalid callback URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::BadRequest(
            "callback URL must be HTTP(S) without userinfo".into(),
        ));
    }
    if allowed.is_some_and(|origins| {
        !origins
            .iter()
            .any(|v| url::Url::parse(v).is_ok_and(|a| a.origin() == url.origin()))
    }) {
        return Err(ApiError::BadRequest(
            "callback origin is not allowed by this server".into(),
        ));
    }
    Ok(())
}

pub(crate) fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        loop {
            while jobs.try_join_next().is_some() {}
            while jobs.len() < 16 {
                match callback_deliveries::claim_due(&state.pool).await {
                    Ok(Some(claim)) => {
                        let state = state.clone();
                        jobs.spawn(async move {
                            deliver(&state, claim).await;
                        });
                    }
                    Ok(None) => break,
                    Err(_) => {
                        tracing::warn!("callback claim storage failure");
                        break;
                    }
                }
            }
            tokio::select! {
                _ = sleep(Duration::from_millis(250)) => {},
                _ = jobs.join_next(), if !jobs.is_empty() => {},
            }
        }
    });
}

async fn deliver(state: &AppState, claim: callback_deliveries::CallbackClaim) {
    let blocked = validate_destination(
        state.config.callback_allowed_origins.as_deref(),
        Some(&claim.url),
    )
    .is_err();
    let status = if blocked {
        None
    } else {
        timeout(Duration::from_secs(10), send(&state.http, &claim))
            .await
            .ok()
            .flatten()
    };
    let result = match status {
        Some(status) if (200..300).contains(&status) => {
            callback_deliveries::complete(&state.pool, &claim, status).await
        }
        _ => callback_deliveries::retry(&state.pool, &claim, status, blocked).await,
    };
    if result.is_err() {
        tracing::warn!("callback result storage failure; lease recovery will retry");
    }
}

async fn send(client: &reqwest::Client, claim: &callback_deliveries::CallbackClaim) -> Option<u16> {
    let mut response = client
        .post(&claim.url)
        .header("Content-Type", "application/json")
        .header("Nod-Delivery-Id", &claim.delivery_id)
        .header("Nod-Event-Id", &claim.event_id)
        .header("Nod-Request-Revision", claim.request_revision.to_string())
        .body(claim.payload_json.clone())
        .send()
        .await
        .ok()?;
    let status = response.status().as_u16();
    // Consume at most 4KiB; never persist or log remote content/URLs.
    let mut remaining = 4096;
    while remaining > 0 {
        match response.chunk().await {
            Ok(Some(chunk)) => remaining -= remaining.min(chunk.len()),
            Ok(None) => break,
            Err(_) => return None,
        }
    }
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outer_timeout_also_bounds_response_body() {
        use axum::{
            body::{Body, Bytes},
            routing::post,
            Router,
        };
        let app = Router::new().route(
            "/",
            post(|| async {
                Body::from_stream(futures_util::stream::once(async {
                    sleep(Duration::from_secs(60)).await;
                    Ok::<_, std::io::Error>(Bytes::from_static(b"late"))
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let claim = callback_deliveries::CallbackClaim {
            delivery_id: "delivery".into(),
            event_id: "event".into(),
            request_revision: 1,
            url,
            payload_json: "{}".into(),
            lease_token: "lease".into(),
            attempts: 1,
        };
        let client = reqwest::Client::new();
        assert!(timeout(Duration::from_millis(100), send(&client, &claim))
            .await
            .is_err());
        server.abort();
    }

    #[tokio::test]
    async fn http_failure_persists_then_retry_succeeds() {
        use axum::{http::StatusCode, routing::post, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let app = Router::new().route(
            "/",
            post(move || {
                let counter = counter.clone();
                async move {
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        StatusCode::NO_CONTENT
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path()).await;
        seed(&state.pool, "failed", "r", 1, &url).await;
        let first = callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .unwrap();
        deliver(&state, first).await;
        let row: (String, i64, i64) =
            sqlx::query_as("SELECT state,attempts,last_status FROM callback_deliveries")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row, ("retrying".into(), 1, 500));
        assert!(callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .is_none());
        sqlx::query("UPDATE callback_deliveries SET next_attempt_at='2020-01-01'")
            .execute(&state.pool)
            .await
            .unwrap();
        let next = callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.payload_json, "{\"revision\":1}");
        deliver(&state, next).await;
        let row: (String, i64, i64) =
            sqlx::query_as("SELECT state,attempts,last_status FROM callback_deliveries")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row, ("delivered".into(), 2, 204));
        server.abort();
    }

    async fn seed(pool: &sqlx::SqlitePool, id: &str, request: &str, revision: i64, url: &str) {
        sqlx::query("INSERT INTO lifecycle_events(event_id,request_id,channel_id,revision,kind,occurred_at,subject,envelope_json,private_detail_json,next_attempt_at) VALUES(?,?,'default',?,'answer','2020-01-01','ev.answer','{}','{}','2020-01-01')")
            .bind(id).bind(request).bind(revision).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO callback_deliveries(delivery_id,event_id,request_id,request_revision,url,payload_json,state,next_attempt_at,updated_at) VALUES(?,?,?,?,?,?,'queued','2020-01-01','2020-01-01')")
            .bind(id).bind(id).bind(request).bind(revision).bind(url).bind(format!("{{\"revision\":{revision}}}")).execute(pool).await.unwrap();
    }

    async fn state(dir: &std::path::Path) -> AppState {
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", dir.join("queue.sqlite").display());
        config.data_dir = dir.into();
        let pool = crate::db::connect(&config).await.unwrap();
        let audit = crate::audit::AuditLogger::new(config.data_dir.clone())
            .await
            .unwrap();
        AppState {
            config: std::sync::Arc::new(config),
            pool,
            audit,
            sync: crate::sync::sender(),
            push: crate::push::PushRegistry::new(vec![]),
            notification_delivery: crate::push::notification_delivery_for_route(None),
            push_route: None,
            publisher_health: Default::default(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    #[tokio::test]
    async fn more_than_sixteen_are_retained_and_same_request_stays_ordered() {
        use axum::{routing::post, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let entered = active.clone();
        let server_gate = gate.clone();
        let app = Router::new().route(
            "/",
            post(move || {
                let gate = server_gate.clone();
                let active = entered.clone();
                async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    gate.acquire().await.unwrap().forget();
                    active.fetch_sub(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path()).await;
        for i in 0..18 {
            seed(
                &state.pool,
                &format!("event{i:02}"),
                &format!("r{i}"),
                1,
                &url,
            )
            .await;
        }
        seed(&state.pool, "later", "r0", 2, &url).await;
        spawn_worker(state.clone());
        timeout(Duration::from_secs(5), async {
            while active.load(Ordering::SeqCst) != 16 {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let queued: i64 =
            sqlx::query_scalar("SELECT count(*) FROM callback_deliveries WHERE state='queued'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(queued, 3);
        let later: String =
            sqlx::query_scalar("SELECT state FROM callback_deliveries WHERE delivery_id='later'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(later, "queued");
        gate.add_permits(19);
        timeout(Duration::from_secs(5), async {
            loop {
                let n: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM callback_deliveries WHERE state='delivered'",
                )
                .fetch_one(&state.pool)
                .await
                .unwrap();
                if n == 19 {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn policy_tighten_blocks_then_revalidates_without_losing_body() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path()).await;
        // No worker races this controlled delivery/policy transition.
        seed(&state.pool, "policy", "r", 1, "http://127.0.0.1:1/").await;
        let claim = callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .unwrap();
        let mut config = (*state.config).clone();
        config.callback_allowed_origins = Some(vec![]);
        state.config = std::sync::Arc::new(config);
        deliver(&state, claim).await;
        let (status, body): (String, String) = sqlx::query_as(
            "SELECT state,payload_json FROM callback_deliveries WHERE delivery_id='policy'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(status, "blocked_policy");
        assert_eq!(body, "{\"revision\":1}");
        assert!(callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .is_none());
        sqlx::query("UPDATE callback_deliveries SET next_attempt_at='2020-01-01'")
            .execute(&state.pool)
            .await
            .unwrap();
        let claim = callback_deliveries::claim_due(&state.pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claim.payload_json, body);
        let mut config = (*state.config).clone();
        config.callback_allowed_origins = None;
        state.config = std::sync::Arc::new(config);
        deliver(&state, claim).await; // refused local port is retryable, no policy block.
        let status: String =
            sqlx::query_scalar("SELECT state FROM callback_deliveries WHERE delivery_id='policy'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(status, "retrying");
    }
    #[tokio::test]
    async fn retry_preserves_frozen_body_and_identity_and_does_not_redirect() {
        use axum::{
            body::Bytes,
            http::{HeaderMap, StatusCode},
            routing::post,
            Router,
        };
        use std::sync::{Arc, Mutex};
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        let app = Router::new()
            .route(
                "/",
                post(move |headers: HeaderMap, body: Bytes| {
                    let captured = captured.clone();
                    async move {
                        let mut rows = captured.lock().unwrap();
                        rows.push((headers, body));
                        if rows.len() == 1 {
                            StatusCode::INTERNAL_SERVER_ERROR
                        } else {
                            StatusCode::NO_CONTENT
                        }
                    }
                }),
            )
            .route(
                "/redirect",
                post(|| async { axum::response::Redirect::temporary("/") }),
            )
            .route("/large", post(|| async { "x".repeat(100_000) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut claim = callback_deliveries::CallbackClaim {
            delivery_id: "delivery".into(),
            event_id: "event".into(),
            request_revision: 2,
            url: format!("http://{address}/"),
            payload_json: "{\"status\":\"pending\",\"decisions\":[]}".into(),
            lease_token: "lease".into(),
            attempts: 1,
        };
        assert_eq!(send(&client, &claim).await, Some(500));
        assert_eq!(send(&client, &claim).await, Some(204));
        {
            let rows = received.lock().unwrap();
            assert_eq!(rows.len(), 2);
            for (headers, body) in rows.iter() {
                assert_eq!(headers["Nod-Delivery-Id"], "delivery");
                assert_eq!(headers["Nod-Event-Id"], "event");
                assert_eq!(headers["Nod-Request-Revision"], "2");
                assert_eq!(body.as_ref(), claim.payload_json.as_bytes());
            }
        }
        claim.url = format!("http://{address}/redirect");
        assert_eq!(send(&client, &claim).await, Some(307));
        assert_eq!(received.lock().unwrap().len(), 2);
        claim.url = format!("http://{address}/large");
        assert_eq!(send(&client, &claim).await, Some(200));
        server.abort();
    }
    #[test]
    fn exact_policy_and_disabled_policy() {
        assert!(validate_destination(None, Some("http://localhost/x")).is_ok());
        assert!(validate_destination(Some(&[]), Some("http://localhost/x")).is_err());
        let allowed = vec!["https://example.com".into()];
        assert!(
            validate_destination(Some(&allowed), Some("https://example.com/x?q=private")).is_ok()
        );
        for url in [
            "http://example.com/x",
            "https://example.com:444/x",
            "https://u:p@example.com/x",
            "ftp://example.com/x",
        ] {
            assert!(validate_destination(Some(&allowed), Some(url)).is_err());
        }
    }
}
