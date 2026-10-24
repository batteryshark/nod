use std::time::Duration;

use nod_apns_relay::error::DeliveryFailure;
use tokio::task::JoinSet;

use crate::{db, models::RequestStatus, state::AppState};
use db::push_deliveries::MAX_ATTEMPTS;
use db::push_deliveries::{self, PushJob, PushProgress};

const PUSH_CONCURRENCY: i64 = 4;
const TASK_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut sends = JoinSet::new();
        let mut recovery = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = recovery.tick() => {},
                Some(result) = sends.join_next(), if !sends.is_empty() => {
                    if let Err(error) = result {
                        tracing::error!(%error, "push worker failed");
                    }
                }
            }
            if let Err(error) = push_deliveries::recover_pushes(&state.pool).await {
                tracing::error!(%error, "failed to recover interrupted push deliveries");
            }
            let available = PUSH_CONCURRENCY - sends.len() as i64;
            if available == 0 {
                continue;
            }
            match push_deliveries::claim_pushes(&state.pool, available).await {
                Ok(jobs) if !jobs.is_empty() => {
                    for job in jobs {
                        let state = state.clone();
                        sends.spawn(async move {
                            deliver_bounded(&state, job, TASK_TIMEOUT).await;
                        });
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "failed to claim queued push deliveries"),
            }
        }
    });
}

async fn deliver_bounded(state: &AppState, job: PushJob, timeout: Duration) {
    // Include preparation and persistence waits, not just the provider call.
    // Leave slack so this future is dropped before its lease can be reclaimed.
    debug_assert!(timeout.as_secs() < push_deliveries::LEASE_SECONDS as u64);
    if tokio::time::timeout(timeout, deliver(state, job))
        .await
        .is_err()
    {
        tracing::error!("push task lifecycle timed out; lease will recover");
    }
}

async fn progress(state: &AppState, update: PushProgress<'_>) {
    if let Err(error) = push_deliveries::update_push(&state.pool, update).await {
        tracing::error!(%error, "failed to record push delivery status");
    }
}

async fn deliver(state: &AppState, job: PushJob) {
    deliver_inner(
        state,
        job,
        #[cfg(test)]
        false,
    )
    .await;
}

async fn deliver_inner(state: &AppState, mut job: PushJob, #[cfg(test)] fail_preparation: bool) {
    while job.attempts < MAX_ATTEMPTS {
        let result = prepare_delivery(state, &job).await;
        #[cfg(test)]
        let result = if fail_preparation {
            Err(sqlx::Error::PoolTimedOut.into())
        } else {
            result
        };
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
            Err(_) => {
                // No send attempt was consumed. Leave the lease recoverable.
                tracing::warn!("push preparation unavailable; awaiting lease recovery");
                return;
            }
        };
        if let Err(error) = push_deliveries::begin_attempt(&state.pool, &mut job).await {
            tracing::error!(%error, "push attempt not recorded; refusing to send");
            return;
        }
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
            match push_deliveries::clear_invalid_push_token(
                &state.pool,
                &job,
                device.push_token.as_deref(),
            )
            .await
            {
                Ok(true) => {
                    let _ = state.sync.send(crate::sync::targeted_envelope(
                        "device_push_updated",
                        serde_json::json!({"device_id":device.id}),
                        vec![device.user_id.clone()],
                    ));
                }
                Ok(false) => {}
                Err(error) => tracing::error!(%error, "failed to clear rejected push token"),
            }
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
        panic_first: bool,
        hang_first: bool,
    }

    impl TestProvider {
        fn new(failures: usize, invalid_token: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                maximum: AtomicUsize::new(0),
                failures,
                invalid_token,
                panic_first: false,
                hang_first: false,
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
            if self.panic_first && call == 0 {
                panic!("simulated push task panic");
            }
            if self.hang_first && call == 0 {
                std::future::pending::<()>().await;
            }
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
            publisher_health: Default::default(),
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
    async fn preparation_error_preserves_budget_and_recovers_after_lease() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        let job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        deliver_inner(&state, job, true).await;
        let result: (String, i64) = sqlx::query_as("SELECT status,attempts FROM push_deliveries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(result, ("sending".into(), 0));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        push_deliveries::recover_pushes(&state.pool).await.unwrap();
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
        assert_eq!(result, ("accepted".into(), 1));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stuck_task_is_bounded_and_its_persisted_attempt_recovers() {
        let mut provider = TestProvider::new(0, false);
        provider.hang_first = true;
        let provider = Arc::new(provider);
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        let job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        deliver_bounded(&state, job, Duration::from_millis(100)).await;
        let result: (String, i64) = sqlx::query_as("SELECT status,attempts FROM push_deliveries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(result, ("sending".into(), 1));
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        push_deliveries::recover_pushes(&state.pool).await.unwrap();
        let job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        deliver_bounded(&state, job, TASK_TIMEOUT).await;
        let result: (String, i64) = sqlx::query_as("SELECT status,attempts FROM push_deliveries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(result, ("accepted".into(), 2));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn panicked_task_is_recovered_by_the_running_worker() {
        let mut provider = TestProvider::new(0, false);
        provider.panic_first = true;
        let provider = Arc::new(provider);
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        spawn_worker(state.clone());
        tokio::time::timeout(Duration::from_secs(3), async {
            while provider.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // Simulate passage of the lease, without making the test wait 90s.
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let result: (String, i64) =
                    sqlx::query_as("SELECT status,attempts FROM push_deliveries")
                        .fetch_one(&state.pool)
                        .await
                        .unwrap();
                if result.0 == "accepted" {
                    assert_eq!(result.1, 2);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn legacy_migration_recovers_jobs_without_losing_enrollment() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider).await;
        queued_request(&state, 2).await;
        sqlx::query("UPDATE push_deliveries SET status='sending',attempts=CASE WHEN device_id='device-0' THEN 3 ELSE 1 END")
            .execute(&state.pool).await.unwrap();
        for column in ["lease_until", "claim_generation"] {
            sqlx::query(&format!("ALTER TABLE push_deliveries DROP COLUMN {column}"))
                .execute(&state.pool)
                .await
                .unwrap();
        }
        state.pool.close().await;
        let pool = db::connect(&state.config).await.unwrap();
        // Reopening again must remain idempotent.
        let second = db::connect(&state.config).await.unwrap();
        push_deliveries::recover_pushes(&pool).await.unwrap();
        let jobs: Vec<(String, i64)> =
            sqlx::query_as("SELECT status,attempts FROM push_deliveries ORDER BY device_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(jobs, vec![("failed".into(), 3), ("queued".into(), 1)]);
        let tokens: i64 =
            sqlx::query_scalar("SELECT count(*) FROM devices WHERE push_token='old-token'")
                .fetch_one(&second)
                .await
                .unwrap();
        assert_eq!(tokens, 2);
    }

    #[tokio::test]
    async fn lease_fences_stale_attempts_and_completions_and_caps_crash_retries() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        let mut stale = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        push_deliveries::begin_attempt(&state.pool, &mut stale)
            .await
            .unwrap();
        push_deliveries::recover_pushes(&state.pool).await.unwrap();
        assert!(push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .is_empty());
        for attempt in 2..=3 {
            sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
                .execute(&state.pool)
                .await
                .unwrap();
            assert!(push_deliveries::update_push(
                &state.pool,
                PushProgress {
                    job: &stale,
                    status: "accepted",
                    error: None
                },
            )
            .await
            .is_err());
            push_deliveries::recover_pushes(&state.pool).await.unwrap();
            let mut current = push_deliveries::claim_pushes(&state.pool, 1)
                .await
                .unwrap()
                .pop()
                .unwrap();
            assert!(current.claim_generation > stale.claim_generation);
            assert!(push_deliveries::update_push(
                &state.pool,
                PushProgress {
                    job: &stale,
                    status: "accepted",
                    error: None
                }
            )
            .await
            .is_err());
            assert!(push_deliveries::begin_attempt(&state.pool, &mut stale)
                .await
                .is_err());
            // A stale task must not reach the provider even if preparation succeeds.
            deliver(&state, stale.clone()).await;
            push_deliveries::begin_attempt(&state.pool, &mut current)
                .await
                .unwrap();
            assert_eq!(current.attempts, attempt);
        }
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        for _ in 0..2 {
            push_deliveries::recover_pushes(&state.pool).await.unwrap();
            assert!(push_deliveries::claim_pushes(&state.pool, 1)
                .await
                .unwrap()
                .is_empty());
        }
        let result: (String, i64) = sqlx::query_as("SELECT status,attempts FROM push_deliveries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(result, ("failed".into(), 3));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn periodic_recovery_runs_without_restart_and_preserves_healthy_lease() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 1).await;
        push_deliveries::claim_pushes(&state.pool, 1).await.unwrap();
        spawn_worker(state.clone());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status: String = sqlx::query_scalar("SELECT status FROM push_deliveries")
                    .fetch_one(&state.pool)
                    .await
                    .unwrap();
                if status == "accepted" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
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
    async fn invalid_token_clear_rejects_expired_reclaimed_and_stale_attempt_owners() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider).await;
        queued_request(&state, 1).await;
        let mut stale = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        push_deliveries::begin_attempt(&state.pool, &mut stale)
            .await
            .unwrap();
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1")
            .execute(&state.pool)
            .await
            .unwrap();
        assert!(
            !push_deliveries::clear_invalid_push_token(&state.pool, &stale, Some("old-token"))
                .await
                .unwrap()
        );
        push_deliveries::recover_pushes(&state.pool).await.unwrap();
        let mut current = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        // Even if the device re-registers identical token text, old ownership
        // cannot invalidate the token under the new claim's healthy lease.
        sqlx::query("UPDATE devices SET push_token='old-token'")
            .execute(&state.pool)
            .await
            .unwrap();
        assert!(
            !push_deliveries::clear_invalid_push_token(&state.pool, &stale, Some("old-token"))
                .await
                .unwrap()
        );
        let prior_attempt = current.clone();
        push_deliveries::begin_attempt(&state.pool, &mut current)
            .await
            .unwrap();
        assert!(!push_deliveries::clear_invalid_push_token(
            &state.pool,
            &prior_attempt,
            Some("old-token")
        )
        .await
        .unwrap());
        push_deliveries::update_push(
            &state.pool,
            PushProgress {
                job: &current,
                status: "accepted",
                error: None,
            },
        )
        .await
        .unwrap();
        assert!(!push_deliveries::clear_invalid_push_token(
            &state.pool,
            &current,
            Some("old-token")
        )
        .await
        .unwrap());
        let token: String = sqlx::query_scalar("SELECT push_token FROM devices")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(token, "old-token");
    }

    #[tokio::test]
    async fn invalid_token_clear_requires_sent_token_and_reports_only_actual_changes() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider).await;
        queued_request(&state, 1).await;
        let mut job = push_deliveries::claim_pushes(&state.pool, 1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        push_deliveries::begin_attempt(&state.pool, &mut job)
            .await
            .unwrap();
        sqlx::query("UPDATE devices SET push_token='fresh-token'")
            .execute(&state.pool)
            .await
            .unwrap();
        assert!(
            !push_deliveries::clear_invalid_push_token(&state.pool, &job, Some("old-token"))
                .await
                .unwrap()
        );
        let token: String = sqlx::query_scalar("SELECT push_token FROM devices")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(token, "fresh-token");
        assert!(
            push_deliveries::clear_invalid_push_token(&state.pool, &job, Some("fresh-token"))
                .await
                .unwrap()
        );
        assert!(
            !push_deliveries::clear_invalid_push_token(&state.pool, &job, Some("fresh-token"))
                .await
                .unwrap()
        );
        assert!(
            !push_deliveries::clear_invalid_push_token(&state.pool, &job, None)
                .await
                .unwrap()
        );
        let token: Option<String> = sqlx::query_scalar("SELECT push_token FROM devices")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(token.is_none());
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
    }

    #[tokio::test]
    async fn worker_bounds_concurrency_and_recovers_interrupted_outbox_jobs() {
        let provider = Arc::new(TestProvider::new(0, false));
        let (_dir, state) = test_state(provider.clone()).await;
        queued_request(&state, 11).await;
        push_deliveries::claim_pushes(&state.pool, 2).await.unwrap();
        sqlx::query("UPDATE push_deliveries SET lease_until=unixepoch()-1 WHERE status='sending'")
            .execute(&state.pool)
            .await
            .unwrap();
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
    async fn recovered_delivery_rechecks_revocation_handling_and_subscription() {
        for change in [
            "UPDATE devices SET revoked_at='2026-01-01T00:00:00Z'",
            "UPDATE requests SET status='cancelled'",
            "UPDATE user_channel_subscriptions SET subscribed=0",
            "UPDATE requests SET expires_at='2020-01-01T00:00:00Z'",
        ] {
            let provider = Arc::new(TestProvider::new(0, false));
            let (_dir, state) = test_state(provider.clone()).await;
            queued_request(&state, 1).await;
            push_deliveries::claim_pushes(&state.pool, 1).await.unwrap();
            sqlx::query("UPDATE push_deliveries SET lease_until=NULL")
                .execute(&state.pool)
                .await
                .unwrap();
            push_deliveries::recover_pushes(&state.pool).await.unwrap();
            sqlx::query(change).execute(&state.pool).await.unwrap();
            let job = push_deliveries::claim_pushes(&state.pool, 1)
                .await
                .unwrap()
                .pop()
                .unwrap();
            deliver(&state, job).await;
            let result: (String, i64) =
                sqlx::query_as("SELECT status,attempts FROM push_deliveries")
                    .fetch_one(&state.pool)
                    .await
                    .unwrap();
            assert_eq!(result, ("skipped".into(), 0));
            assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        }
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
