//! A single bounded publisher; SQLite remains authoritative while the bus is down.
use crate::{db::lifecycle, state::AppState};
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::time::{sleep, timeout};

#[derive(Clone)]
pub(crate) struct PublisherHealth(Arc<RwLock<&'static str>>);
impl Default for PublisherHealth {
    fn default() -> Self {
        Self(Arc::new(RwLock::new("disabled")))
    }
}
impl PublisherHealth {
    pub fn status(&self) -> &'static str {
        self.0.read().map(|v| *v).unwrap_or("unhealthy")
    }
    fn set(&self, value: &'static str) {
        if let Ok(mut v) = self.0.write() {
            *v = value;
        }
    }
}

pub(crate) fn spawn_worker(state: AppState) {
    if !state.config.integrations.nats.configured() {
        return;
    }
    state.publisher_health.set("connecting");
    tokio::spawn(async move {
        let config = &state.config.integrations.nats;
        loop {
            if !config.valid() {
                state.publisher_health.set("configuration_error");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
            let connection = timeout(Duration::from_secs(10), async {
                let options = connection_options(config).await?;
                options
                    .max_reconnects(Some(0))
                    .connect(config.url.as_deref().unwrap())
                    .await
                    .map_err(|_| "unavailable")
            })
            .await;
            let client = match connection {
                Ok(Ok(client)) => client,
                Ok(Err(status)) => {
                    state.publisher_health.set(status);
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
                _ => {
                    state.publisher_health.set("unavailable");
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            let context = async_nats::jetstream::new(client.clone());
            state.publisher_health.set("connected");
            loop {
                if client.connection_state() != async_nats::connection::State::Connected {
                    state.publisher_health.set("unavailable");
                    break;
                }
                let claim = match lifecycle::claim_due(&state.pool).await {
                    Ok(Some(claim)) => claim,
                    Ok(None) => {
                        sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                    Err(_) => {
                        state.publisher_health.set("storage_error");
                        sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                let sequence = publish(&context, &claim, &config.expected_stream).await;
                let result = if let Some(sequence) = sequence {
                    state.publisher_health.set("connected");
                    lifecycle::published(&state.pool, &claim, sequence).await
                } else {
                    state.publisher_health.set("publish_error");
                    lifecycle::retry(&state.pool, &claim).await
                };
                if result.is_err() {
                    state.publisher_health.set("storage_error");
                }
                if sequence.is_none() {
                    break;
                }
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
}

// Read authentication files only at connection setup, never at config load or
// on the API path. Errors are static codes: file/transport errors can contain secrets.
async fn connection_options(
    config: &crate::config::NatsConfig,
) -> Result<async_nats::ConnectOptions, &'static str> {
    if !config.valid() {
        return Err("configuration_error");
    }
    let options = if let Some(path) = config.token_file.as_ref().or(config.password_file.as_ref()) {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|_| "authentication_file_error")?;
        let token = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(&raw);
        if token.is_empty() || token.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("authentication_file_error");
        }
        if let Some(user) = &config.user {
            Ok(async_nats::ConnectOptions::new().user_and_password(user.clone(), token.to_owned()))
        } else {
            Ok(async_nats::ConnectOptions::new().token(token.to_owned()))
        }
    } else {
        async_nats::ConnectOptions::with_credentials_file(
            config
                .credentials_file
                .as_ref()
                .ok_or("configuration_error")?,
        )
        .await
        .map_err(|_| "authentication_file_error")
    }?;
    Ok(if let Some(prefix) = &config.inbox_prefix {
        options.custom_inbox_prefix(prefix.clone())
    } else {
        options
    })
}

async fn publish(
    context: &async_nats::jetstream::Context,
    claim: &lifecycle::PublishClaim,
    expected: &str,
) -> Option<i64> {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Nats-Msg-Id", claim.event_id.as_str());
    headers.insert("Nats-Expected-Stream", expected);
    timeout(Duration::from_secs(10), async {
        let ack = context
            .publish_with_headers(
                claim.subject.clone(),
                headers,
                claim.envelope_json.clone().into(),
            )
            .await
            .ok()?
            .await
            .ok()?;
        acknowledged_sequence(expected, &ack.stream, ack.sequence)
    })
    .await
    .ok()
    .flatten()
}

fn acknowledged_sequence(expected: &str, stream: &str, sequence: u64) -> Option<i64> {
    (expected == stream)
        .then(|| i64::try_from(sequence).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires nats-server on PATH or NOD_TEST_NATS_SERVER; run: cargo test --locked -j2 -p nod-server --lib services::lifecycle_publish::tests::isolated_jetstream_exact_nod_acl -- --ignored --exact"]
    async fn isolated_jetstream_exact_nod_acl() {
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config_path = dir.path().join("server.conf");
        std::fs::write(&config_path, format!(r#"
host: "127.0.0.1"
port: {port}
jetstream {{ store_dir: "{}" }}
authorization {{ users: [
 {{ user: "admin", password: "local-admin" }},
 {{ user: "nod", password: "local-password", permissions: {{ publish: ["ev.nod.>"], subscribe: ["_INBOX_nod.>"] }} }}
] }}
"#, dir.path().join("store").display())).unwrap();
        drop(listener);
        let executable =
            std::env::var_os("NOD_TEST_NATS_SERVER").unwrap_or_else(|| "nats-server".into());
        let _server = Server(
            std::process::Command::new(&executable)
                .arg("-c")
                .arg(&config_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap_or_else(|error| {
                    panic!("could not launch test NATS executable {executable:?}: {error}; set NOD_TEST_NATS_SERVER to a nats-server executable or put nats-server on PATH")
                }),
        );
        timeout(Duration::from_secs(15), async {
            let url = format!("nats://127.0.0.1:{port}");
            let admin = loop {
                if let Ok(client) = async_nats::ConnectOptions::new()
                    .user_and_password("admin".into(), "local-admin".into())
                    .connect(&url)
                    .await
                {
                    break client;
                }
                sleep(Duration::from_millis(50)).await;
            };
            async_nats::jetstream::new(admin)
                .create_stream(async_nats::jetstream::stream::Config {
                    name: "TOWER".into(),
                    subjects: vec!["ev.>".into()],
                    ..Default::default()
                })
                .await
                .unwrap();
            let path = dir.path().join("password");
            tokio::fs::write(&path, "local-password\r\n").await.unwrap();
            let config = crate::config::NatsConfig {
                url: Some(url.clone()),
                user: Some("nod".into()),
                password_file: Some(path),
                inbox_prefix: Some("_INBOX_nod".into()),
                ..Default::default()
            };
            let (errors_tx, mut errors_rx) = tokio::sync::mpsc::unbounded_channel();
            let client = connection_options(&config)
                .await
                .unwrap()
                .event_callback(move |event| {
                    let errors_tx = errors_tx.clone();
                    async move {
                        if let async_nats::Event::ServerError(error) = event {
                            let _ = errors_tx.send(error.to_string());
                        }
                    }
                })
                .connect(&url)
                .await
                .unwrap();
            let context = async_nats::jetstream::new(client.clone());
            let claim = lifecycle::PublishClaim {
                event_id: "local-event".into(),
                subject: "ev.nod.request.created".into(),
                envelope_json: "{}".into(),
                lease_token: "local".into(),
                attempts: 0,
            };
            assert_eq!(publish(&context, &claim, "TOWER").await, Some(1));
            assert_eq!(publish(&context, &claim, "WRONG").await, None);
            client
                .publish("ev.other.test", "forbidden".into())
                .await
                .unwrap();
            let _forbidden = client.subscribe("_INBOX.forbidden").await.unwrap();
            client.flush().await.unwrap();
            let first = errors_rx.recv().await.unwrap();
            let second = errors_rx.recv().await.unwrap();
            assert!(first.contains("Permissions") || first.contains("permissions"));
            assert!(second.contains("Permissions") || second.contains("permissions"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn auth_token_file_errors_are_static_and_nonblocking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private-dummy-token");
        let mut config = crate::config::NatsConfig {
            url: Some("nats://localhost:4222".into()),
            token_file: Some(path.clone()),
            ..Default::default()
        };
        assert_eq!(
            connection_options(&config).await.err(),
            Some("authentication_file_error")
        );
        for content in [
            "",
            "\n",
            "dummy\nsecond",
            "dummy\n\n",
            "dummy\r",
            "dummy\0token",
            " dummy",
            "dummy token",
        ] {
            tokio::fs::write(&path, content).await.unwrap();
            assert_eq!(
                connection_options(&config).await.err(),
                Some("authentication_file_error")
            );
        }
        for content in ["dummy", "dummy\n", "dummy\r\n"] {
            tokio::fs::write(&path, content).await.unwrap();
            assert!(connection_options(&config).await.is_ok());
        }
        config.credentials_file = Some(dir.path().join("also-private"));
        assert_eq!(
            connection_options(&config).await.err(),
            Some("configuration_error")
        );
        config.token_file = None;
        assert_eq!(
            connection_options(&config).await.err(),
            Some("authentication_file_error")
        );
        config.credentials_file = None;
        assert_eq!(
            connection_options(&config).await.err(),
            Some("configuration_error")
        );

        // Invalid authentication does not enter startup's failure path or read a file there.
        let mut server_config = crate::config::Config::with_admin_token("test");
        server_config.database_url =
            format!("sqlite://{}", dir.path().join("auth.sqlite").display());
        server_config.data_dir = dir.path().into();
        server_config.integrations.nats = config;
        let state = timeout(Duration::from_secs(5), AppState::new(server_config))
            .await
            .unwrap()
            .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(state.publisher_health.status(), "configuration_error");
    }

    #[tokio::test]
    async fn auth_token_and_creds_routes_emit_expected_dummy_connect_fields() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        for credentials_route in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("dummy-auth-file");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let (reader, mut writer) = socket.into_split();
                let mut reader = BufReader::new(reader);
                writer.write_all(b"INFO {\"server_id\":\"mock\",\"version\":\"2.10.0\",\"proto\":1,\"auth_required\":true,\"nonce\":\"dummy-nonce\",\"max_payload\":1048576}\r\n").await.unwrap();
                let mut connect = None;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).await.unwrap() > 0);
                    if let Some(json) = line.strip_prefix("CONNECT ") {
                        connect = Some(serde_json::from_str::<serde_json::Value>(json).unwrap());
                    }
                    if line == "PING\r\n" {
                        writer.write_all(b"PONG\r\n").await.unwrap();
                        return connect.unwrap();
                    }
                }
            });
            let mut config = crate::config::NatsConfig {
                url: Some(format!("nats://{address}")),
                ..Default::default()
            };
            if credentials_route {
                // Public async-nats API-documentation example seed, never a real credential.
                let dummy = "-----BEGIN NATS USER JWT-----\ndummy.jwt.value\n------END NATS USER JWT------\n-----BEGIN USER NKEY SEED-----\nSUAIO3FHUX5PNV2LQIIP7TZ3N4L7TX3W53MQGEIVYFIGA635OZCKEYHFLM\n------END USER NKEY SEED------\n";
                tokio::fs::write(&path, dummy).await.unwrap();
                config.credentials_file = Some(path);
            } else {
                tokio::fs::write(&path, "dummy-local-token\n")
                    .await
                    .unwrap();
                config.token_file = Some(path);
            }
            let options = connection_options(&config).await.unwrap();
            let client = timeout(
                Duration::from_secs(5),
                options
                    .max_reconnects(Some(0))
                    .connect(config.url.as_deref().unwrap()),
            )
            .await
            .unwrap()
            .unwrap();
            let connect = timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            if credentials_route {
                assert_eq!(connect["jwt"], "dummy.jwt.value");
                assert!(connect["sig"].as_str().is_some_and(|v| !v.is_empty()));
                assert!(connect["auth_token"].is_null());
            } else {
                assert_eq!(connect["auth_token"], "dummy-local-token");
                assert!(connect["jwt"].is_null());
            }
            drop(client);
        }
    }
    #[tokio::test]
    async fn local_wire_mock_checks_exact_bytes_headers_wrong_stream_and_duplicate_ack() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            let mut reader = BufReader::new(reader);
            writer.write_all(b"INFO {\"server_id\":\"mock\",\"server_name\":\"local\",\"version\":\"2.10.0\",\"proto\":1,\"headers\":true,\"max_payload\":1048576}\r\n").await.unwrap();
            let mut sid = String::new();
            let mut publishes = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let fields: Vec<_> = line.split_whitespace().collect();
                match fields.first().copied() {
                    Some("PING") => writer.write_all(b"PONG\r\n").await.unwrap(),
                    Some("SUB") => sid = fields.last().unwrap().to_string(),
                    Some("HPUB") => {
                        let length: usize = fields.last().unwrap().parse().unwrap();
                        let header_length: usize = fields[fields.len() - 2].parse().unwrap();
                        let mut payload = vec![0; length + 2];
                        reader.read_exact(&mut payload).await.unwrap();
                        tx.send((
                            fields[1].to_string(),
                            header_length,
                            payload[..length].to_vec(),
                        ))
                        .await
                        .unwrap();
                        let stream = if publishes == 0 { "WRONG" } else { "TOWER" };
                        let ack =
                            format!("{{\"stream\":\"{stream}\",\"seq\":42,\"duplicate\":true}}");
                        writer
                            .write_all(
                                format!("MSG {} {} {}\r\n{}\r\n", fields[2], sid, ack.len(), ack)
                                    .as_bytes(),
                            )
                            .await
                            .unwrap();
                        publishes += 1;
                    }
                    _ => {}
                }
            }
        });
        let client = timeout(
            Duration::from_secs(5),
            async_nats::connect(format!("nats://{address}")),
        )
        .await
        .unwrap()
        .unwrap();
        let context = async_nats::jetstream::new(client);
        let claim = lifecycle::PublishClaim {
            event_id: "stable-event".into(),
            subject: "ev.nod.answer.recorded".into(),
            envelope_json: "{ \"frozen\": true }".into(),
            lease_token: "lease".into(),
            attempts: 1,
        };
        assert_eq!(publish(&context, &claim, "TOWER").await, None);
        assert_eq!(publish(&context, &claim, "TOWER").await, Some(42));
        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        // HeaderMap order is intentionally unspecified; values and body are frozen.
        for wire in [first, second] {
            assert_eq!(wire.0, claim.subject);
            let headers = String::from_utf8_lossy(&wire.2[..wire.1]);
            assert!(headers.contains("Nats-Msg-Id: stable-event\r\n"));
            assert!(headers.contains("Nats-Expected-Stream: TOWER\r\n"));
            assert_eq!(&wire.2[wire.1..], claim.envelope_json.as_bytes());
        }
        server.abort();
    }
    #[tokio::test]
    async fn disabled_or_misconfigured_publisher_does_not_block_startup() {
        for configured in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut config = crate::config::Config::with_admin_token("test");
            config.data_dir = dir.path().into();
            config.database_url =
                format!("sqlite://{}", dir.path().join("startup.sqlite").display());
            if configured {
                config.integrations.nats.url = Some("not-a-valid-url".into());
            }
            let state = timeout(Duration::from_secs(5), AppState::new(config))
                .await
                .unwrap()
                .unwrap();
            tokio::task::yield_now().await;
            assert_eq!(
                state.publisher_health.status(),
                if configured {
                    "configuration_error"
                } else {
                    "disabled"
                }
            );
            assert_eq!(
                crate::db::lifecycle::high_water(&state.pool).await.unwrap(),
                0
            );
        }
    }
    #[test]
    fn wrong_stream_or_overflow_is_not_success_duplicate_ack_is_success() {
        assert_eq!(acknowledged_sequence("TOWER", "OTHER", 1), None);
        assert_eq!(acknowledged_sequence("TOWER", "TOWER", u64::MAX), None);
        // Duplicate PubAck has the same stream/sequence; deliberately not rejected.
        assert_eq!(acknowledged_sequence("TOWER", "TOWER", 10), Some(10));
    }

    #[tokio::test]
    async fn outage_restart_keeps_frozen_identity_and_fences_expired_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", dir.path().join("queue.sqlite").display());
        config.data_dir = dir.path().into();
        let pool = crate::db::connect(&config).await.unwrap();
        let frozen = "{\"id\":\"stable\",\"time\":\"2020-01-01T00:00:00Z\"}";
        sqlx::query("INSERT INTO lifecycle_events(event_id,request_id,channel_id,revision,kind,occurred_at,subject,envelope_json,private_detail_json,next_attempt_at) VALUES('stable','r','default',1,'created','2020-01-01','ev.nod.request.created',?,'{}','2020-01-01')")
            .bind(frozen).execute(&pool).await.unwrap();
        let first = lifecycle::claim_due(&pool).await.unwrap().unwrap();
        assert!(lifecycle::claim_due(&pool).await.unwrap().is_none());
        assert!(lifecycle::retry(&pool, &first).await.unwrap());
        assert!(lifecycle::claim_due(&pool).await.unwrap().is_none());
        sqlx::query("UPDATE lifecycle_events SET next_attempt_at='2020-01-01'")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = crate::db::connect(&config).await.unwrap();
        let second = lifecycle::claim_due(&pool).await.unwrap().unwrap();
        assert_eq!(second.event_id, first.event_id);
        assert_eq!(second.subject, first.subject);
        assert_eq!(second.envelope_json, frozen);
        assert_ne!(second.lease_token, first.lease_token);
        assert!(!lifecycle::published(&pool, &first, 1).await.unwrap());
        sqlx::query("UPDATE lifecycle_events SET lease_until='2020-01-01'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!lifecycle::published(&pool, &second, 1).await.unwrap());
        let third = lifecycle::claim_due(&pool).await.unwrap().unwrap();
        assert_eq!(third.event_id, "stable");
        assert_eq!(third.envelope_json, frozen);
        assert!(lifecycle::published(&pool, &third, 99).await.unwrap());
        assert!(lifecycle::claim_due(&pool).await.unwrap().is_none());
    }
}
