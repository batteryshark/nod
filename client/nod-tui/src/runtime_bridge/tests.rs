use nod_client_core::{EnrollParams, SubmitOptionParams};

use super::{execute_runtime_command, RuntimeCommand, RuntimeCommandOutcome};
use crate::test_support::FakeRuntime;

#[tokio::test]
async fn enroll_connects_sync_after_success() {
    let mut runtime = FakeRuntime::default();

    let outcome = execute_runtime_command(
        &mut runtime,
        RuntimeCommand::Enroll(EnrollParams {
            base_url: "http://localhost:8767".to_string(),
            device_name: "terminal".to_string(),
            code: "ABCDEFGH".to_string(),
            notification_sound: Some("default".to_string()),
            platform: None,
            native_app_id: None,
            push_provider: None,
            push_token: None,
            attestation: None,
        }),
    )
    .await;

    assert!(matches!(outcome, Ok(RuntimeCommandOutcome::State(_))));
    assert_eq!(runtime.calls, vec!["enroll", "connect_sync"]);
}

#[tokio::test]
async fn enrollment_remains_successful_when_sync_cannot_start() {
    let mut runtime = FakeRuntime {
        fail_connect: true,
        ..FakeRuntime::default()
    };
    let outcome = execute_runtime_command(
        &mut runtime,
        RuntimeCommand::Enroll(EnrollParams {
            base_url: "http://localhost:8767".into(),
            device_name: "terminal".into(),
            code: "ABCDEFGH".into(),
            notification_sound: None,
            platform: None,
            native_app_id: None,
            push_provider: None,
            push_token: None,
            attestation: None,
        }),
    )
    .await
    .unwrap();
    let RuntimeCommandOutcome::State(state) = outcome else {
        panic!("expected enrolled state")
    };
    assert!(state.is_registered);
    assert!(state.last_error.unwrap().contains("Enrollment completed"));
}

#[tokio::test]
async fn submit_option_surfaces_runtime_errors() {
    let mut runtime = FakeRuntime {
        fail_submit: true,
        ..FakeRuntime::default()
    };

    let outcome = execute_runtime_command(
        &mut runtime,
        RuntimeCommand::SubmitOption(SubmitOptionParams {
            request_id: "missing".to_string(),
            option_id: "approve".to_string(),
            text: None,
        }),
    )
    .await;

    assert_eq!(
        outcome.err().map(|error| error.to_string()),
        Some("stale request".to_string())
    );
    assert_eq!(runtime.calls, vec!["submit_option"]);
}
