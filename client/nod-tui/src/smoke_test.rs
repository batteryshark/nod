//! Event-loop smoke test: enroll → render → submit → state update, driven
//! entirely through key events against a [`FakeRuntime`]. This pins the wiring
//! between the key handler, the runtime commands, and the rendered screen —
//! the seams a real session exercises — without a terminal or a server.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nod_client_core::models::{OptionKind, RequestOption, RequestStatus};
use ratatui::{backend::TestBackend, Terminal};

use crate::{
    app::{AppState, Modal},
    runtime_bridge::{execute_runtime_command, RuntimeCommand},
    test_support::{client_state, request_with_status, FakeRuntime},
    ui::render,
};

#[tokio::test]
async fn event_loop_smoke_enroll_render_submit() {
    // An unregistered session boots into the enrollment modal.
    let mut boot_state = client_state();
    boot_state.is_registered = false;
    boot_state.servers.clear();
    boot_state.requests.clear();
    boot_state.selected_request_id = None;
    let mut app = AppState::new(boot_state);
    assert!(matches!(app.modal(), Some(Modal::Enrollment(_))));

    // Fill the form via key events: server URL, Tab past the prefilled device
    // name, Tab to the code field, type the code, Enter to submit.
    type_text(&mut app, "http://localhost:8767");
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "ABCDEFGH");
    let commands = app.handle_key(key(KeyCode::Enter));
    let [RuntimeCommand::Enroll(params)] = commands.as_slice() else {
        panic!("enter on a completed form should emit exactly Enroll, got {commands:?}");
    };
    assert_eq!(params.base_url, "http://localhost:8767");
    assert_eq!(params.code, "ABCDEFGH");

    // The runtime enrolls (and connects sync), returning a registered state
    // whose selected request offers an approve option.
    let mut runtime = FakeRuntime {
        state: approvable_state(),
        submit_result: resolved_request(),
        ..FakeRuntime::default()
    };
    for command in commands {
        app.begin_command(&command);
        let outcome = execute_runtime_command(&mut runtime, command)
            .await
            .expect("enrollment against the fake runtime succeeds");
        app.apply_runtime_outcome(outcome);
    }
    assert_eq!(runtime.calls, vec!["enroll", "connect_sync"]);
    assert!(app.is_registered());
    assert!(app.modal().is_none(), "enrollment modal closes on success");

    // The pending request renders on the main screen.
    let screen = render_to_text(&app);
    assert!(
        screen.contains("deploy"),
        "pending request title should be on screen:\n{screen}"
    );

    // 'a' approves the selected request through the runtime, and the resolved
    // request lands back in the rendered state.
    let commands = app.handle_key(key(KeyCode::Char('a')));
    let [RuntimeCommand::SubmitOption(params)] = commands.as_slice() else {
        panic!("'a' should emit exactly SubmitOption, got {commands:?}");
    };
    assert_eq!(params.request_id, "deploy");
    assert_eq!(params.option_id, "approve");
    assert_eq!(
        params.expected_request_digest, "digest",
        "the submit is bound to the digest of the rendered request"
    );
    for command in commands {
        app.begin_command(&command);
        let outcome = execute_runtime_command(&mut runtime, command)
            .await
            .expect("submit against the fake runtime succeeds");
        app.apply_runtime_outcome(outcome);
    }
    assert!(runtime.calls.contains(&"submit_option"));
    assert_eq!(
        app.client_state().requests[0].status,
        RequestStatus::Resolved,
        "the submitted decision replaces the cached request"
    );
    let screen = render_to_text(&app);
    assert!(
        screen.contains("resolved"),
        "resolved status should render:\n{screen}"
    );
}

fn approvable_state() -> nod_client_core::models::ClientState {
    let mut state = client_state();
    state.requests[0].options.push(approve_option());
    state
}

fn resolved_request() -> nod_client_core::models::Request {
    let mut resolved = request_with_status("deploy", "default", RequestStatus::Resolved);
    resolved.options.push(approve_option());
    resolved
}

fn approve_option() -> RequestOption {
    RequestOption {
        id: "approve".to_string(),
        label: "Approve".to_string(),
        kind: OptionKind::Approve,
        style: "primary".to_string(),
        requires_text: false,
        text_placeholder: None,
        destructive: false,
        foreground: false,
    }
}

fn render_to_text(app: &AppState) -> String {
    let backend = TestBackend::new(100, 32);
    let mut terminal = Terminal::new(backend).expect("test backend initializes");
    terminal
        .draw(|frame| render(frame, app))
        .expect("render completes");
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

fn type_text(app: &mut AppState, text: &str) {
    for ch in text.chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
}

fn press(app: &mut AppState, code: KeyCode) {
    app.handle_key(key(code));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
