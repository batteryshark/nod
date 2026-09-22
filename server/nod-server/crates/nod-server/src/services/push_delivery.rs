use std::time::Duration;

use nod_apns_relay::error::DeliveryFailure;
use tokio::task::JoinSet;

use crate::{db, models::RequestStatus, state::AppState};
use db::push_deliveries::{self, PushJob, PushProgress};

const PUSH_CONCURRENCY: i64 = 4;
const MAX_ATTEMPTS: i64 = 3;

pub(crate) fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        if let Err(error) = push_deliveries::recover_pushes(&state.pool).await {
            tracing::error!(%error, "failed to recover interrupted push deliveries");
        }
        loop {
            match push_deliveries::claim_pushes(&state.pool, PUSH_CONCURRENCY).await {
                Ok(jobs) if !jobs.is_empty() => {
                    let mut sends = JoinSet::new();
                    for job in jobs {
                        let state = state.clone();
                        sends.spawn(async move {
                            deliver(&state, job).await;
                        });
                    }
                    while let Some(result) = sends.join_next().await {
                        if let Err(error) = result {
                            tracing::error!(%error, "push worker failed");
                        }
                    }
                    continue;
                }
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "failed to claim queued push deliveries"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

async fn progress(state: &AppState, update: PushProgress<'_>) {
    if let Err(error) = push_deliveries::update_push(&state.pool, update).await {
        tracing::error!(%error, "failed to record push delivery status");
    }
}

async fn deliver(state: &AppState, mut job: PushJob) {
    while job.attempts < MAX_ATTEMPTS {
        let result = prepare_delivery(state, &job).await;
        let (device, request) = match result {
            Ok(Some(target)) => target,
            Ok(None) => {
                progress(
                    state,
                    PushProgress {
                        job: &job,
                        status: "skipped",
                        error: Some("request handled, alerts muted/snoozed, device revoked, or push route unavailable"),
                    },
                )
                .await;
                return;
            }
            Err(error) => {
                progress(
                    state,
                    PushProgress {
                        job: &job,
                        status: "failed",
                        error: Some(&error.to_string()),
                    },
                )
                .await;
                return;
            }
        };
        job.attempts += 1;
        progress(
            state,
            PushProgress {
                job: &job,
                status: "sending",
                error: None,
            },
        )
        .await;
        let sent = tokio::time::timeout(
            Duration::from_secs(15),
            state.push.push_request(&device, &request),
        )
        .await;
        let error = match sent {
            Ok(Ok(())) => {
                progress(
                    state,
                    PushProgress {
                        job: &job,
                        status: "accepted",
                        error: None,
                    },
                )
                .await;
                return;
            }
            Ok(Err(error)) => error,
            Err(_) => DeliveryFailure {
                message: "push delivery timed out".to_string(),
                retryable: true,
                invalid_token: false,
            }
            .into(),
        };
        let failure = error.downcast_ref::<DeliveryFailure>();
        let retryable = failure.map(|error| error.retryable).unwrap_or_else(|| {
            error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
            })
        });
        if failure.is_some_and(|error| error.invalid_token) {
            if let Err(error) = push_deliveries::clear_invalid_push_token(
                &state.pool,
                &device.id,
                device.push_token.as_deref(),
            )
            .await
            {
                tracing::error!(%error, "failed to clear rejected push token");
            }
            let _ = state.sync.send(crate::sync::targeted_envelope(
                "device_push_updated",
                serde_json::json!({"device_id":device.id}),
                vec![device.user_id.clone()],
            ));
        }
        let message: String = error.to_string().chars().take(240).collect();
        let will_retry = retryable && job.attempts < MAX_ATTEMPTS;
        progress(
            state,
            PushProgress {
                job: &job,
                status: if will_retry { "sending" } else { "failed" },
                error: Some(&message),
            },
        )
        .await;
        if !will_retry {
            return;
        }
        // Keep the worker slot during backoff so retries cannot cause an
        // unbounded task fanout when Apple's service is unavailable.
        tokio::time::sleep(Duration::from_secs(job.attempts as u64)).await;
    }
    progress(
        state,
        PushProgress {
            job: &job,
            status: "failed",
            error: Some("delivery attempt limit reached"),
        },
    )
    .await;
}

async fn prepare_delivery(
    state: &AppState,
    job: &PushJob,
) -> Result<Option<(crate::models::Device, crate::models::DecisionRequest)>, crate::error::ApiError>
{
    let device = match db::get_device(&state.pool, &job.device_id).await {
        Ok(device) => device,
        Err(crate::error::ApiError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !state.push.has_route(&device) || device.push_token.is_none() {
        return Ok(None);
    }
    if !db::request_delivery_eligible(&state.pool, &job.request_id, &device.user_id).await? {
        return Ok(None);
    }
    let request = match db::request_for_user(&state.pool, &job.request_id, &device.user_id).await {
        Ok(request) => request,
        Err(crate::error::ApiError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !device
        .notification_preferences
        .allows_alert(&request.channel_id, chrono::Utc::now())
    {
        return Ok(None);
    }
    if request.status != RequestStatus::Pending
        || request
            .expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now())
    {
        return Ok(None);
    }
    Ok(Some((device, request)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audit::AuditLogger,
        config::Config,
        models::{DecisionRequest, Device},
        push::{notification_delivery_for_route, PushProvider, PushRegistry},
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct TestProvider {
        calls: AtomicUsize,
        active: AtomicUsize,
        maximum: AtomicUsize,
        failures: usize,
        invalid_token: bool,
    }

    impl TestProvider {
        fn new(failures: usize, invalid_token: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                maximum: AtomicUsize::new(0),
                failures,
                invalid_token,
            }
        }
    }

    #[async_trait]
    impl PushProvider for TestProvider {
        fn id(&self) -> &str {
            "test"
        }
        fn native_app_id(&self) -> Option<&str> {
            Some("test.bundle")
        }
        async fn push_request(&self, _: &Device, _: &DecisionRequest) -> anyhow::Result<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            if call < self.failures {
                return Err(DeliveryFailure {
                    message: "simulated rejection".to_string(),
                    retryable: !self.invalid_token,
                    invalid_token: self.invalid_token,
                }
                .into());
            }
            Ok(())
        }
    }

    async fn test_state(provider: Arc<TestProvider>) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::with_admin_token("test");
        config.database_url = format!("sqlite://{}", dir.path().join("nod.sqlite").display());
        config.data_dir = dir.path().to_path_buf();
        let pool = db::connect(&config).await.unwrap();
        let audit = AuditLogger::new(config.data_dir.clone()).await.unwrap();
        let state = AppState {
            config: Arc::new(config),
            pool,
            audit,
            sync: crate::sync::sender(),
            push: PushRegistry::new(vec![provider]),
            notification_delivery: notification_delivery_for_route(None),
            push_route: None,
            http: reqwest::Client::new(),
            callback_slots: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        (dir, state)
    }

    async fn queued_request(state: &AppState, devices: usize) -> String {
        for index in 0..devices {
            sqlx::query("INSERT INTO devices(id,user_id,name,platform,native_app_id,token_hash,push_provider,push_token,last_seen_at,created_at) VALUES(?,'owner','Phone','ios','test.bundle',?,'test','old-token',?,?)")
                .bind(format!("device-{index}")).bind(format!("hash-{index}")).bind(db::now_string()).bind(db::now_string()).execute(&state.pool).await.unwrap();
        }
        db::create_request(
            &state.pool,
            serde_json::from_value(json!({"title":"Notify"})).unwrap(),
            db::CreateRequestMetadata::default(),
        )
        .await
        .unwrap()
        .request_id
    }

    #[tokio::test]
    async fn transient_push_failures_retry_with_a_persisted_attempt_limit() {
        let provider = Arc::new(TestProvider::new(2, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        let job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        deliver(&state, job).await;
        let result: (String, i64) = sqlx::query_as("SELECT status,attempts FROM push_deliveries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(result, ("accepted".to_string(), 3));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn permanent_invalid_token_stops_retry_and_clears_only_the_rejected_token() {
        let provider = Arc::new(TestProvider::new(5, true));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        let job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        deliver(&state, job).await;
        let token: Option<String> = sqlx::query_scalar("SELECT push_token FROM devices")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(token.is_none());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        sqlx::query("UPDATE devices SET push_token='fresh-token'")
            .execute(&state.pool)
            .await
            .unwrap();
        push_deliveries::clear_invalid_push_token(&state.pool, "device-0", Some("old-token"))
            .await
            .unwrap();
        let token: String = sqlx::query_scalar("SELECT push_token FROM devices")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(token, "fresh-token");
    }

    #[tokio::test]
    async fn worker_bounds_concurrency_and_recovers_interrupted_outbox_jobs() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 11).await;
        push_deliveries::claim_pushes(&state.pool, 2).await.unwrap();
        spawn_worker(state.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let accepted: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM push_deliveries WHERE status='accepted'",
                )
                .fetch_one(&state.pool)
                .await
                .unwrap();
                if accepted == 11 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 11);
        assert!(provider.maximum.load(Ordering::SeqCst) <= PUSH_CONCURRENCY as usize);
    }
    #[tokio::test]
    async fn queued_delivery_rechecks_mute_and_snooze_before_sending() {
        for preferences in [
            nod_proto::DeviceNotificationPreferences {
                muted_channels: vec!["default".to_string()],
                ..Default::default()
            },
            nod_proto::DeviceNotificationPreferences {
                snoozed_until: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                ..Default::default()
            },
        ] {
            let provider = Arc::new(TestProvider::new(0, false));
            let (_directory, state) = test_state(provider.clone()).await;
            queued_request(&state, 1).await;
            db::update_notification_preferences(&state.pool, "device-0", preferences)
                .await
                .unwrap();
            let job = push_deliveries::claim_pushes(&state.pool, 1)
                .await
                .unwrap()
                .pop()
                .unwrap();
            deliver(&state, job).await;
            let status: String = sqlx::query_scalar("SELECT status FROM push_deliveries")
                .fetch_one(&state.pool)
                .await
                .unwrap();
            assert_eq!(status, "skipped");
            assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        }
    }
}
