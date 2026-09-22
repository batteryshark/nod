use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

use crate::{app::AppState, test_support::client_state};

use super::render;

#[test]
fn renders_registered_main_screen() {
    render_app(&AppState::new(client_state()));
}

#[test]
fn renders_enrollment_screen() {
    let mut state = client_state();
    state.is_registered = false;

    render_app(&AppState::new(state));
}

#[test]
fn enrolling_another_server_displays_form_and_failed_draft() {
    let mut app = AppState::new(client_state());
    app.handle_key(key(KeyCode::Char('e')));
    for character in "https://second.example".chars() {
        app.handle_key(key(KeyCode::Char(character)));
    }
    app.set_error("Code expired".into());
    let screen = render_app(&app);
    assert!(screen.contains("Server: https://second.example"));
    assert!(screen.contains("Code expired"));
}

#[test]
fn renders_settings_modal() {
    let mut app = AppState::new(client_state());

    app.handle_key(key(KeyCode::Char(',')));

    render_app(&app);
}

#[test]
fn renders_empty_device_list() {
    let mut app = AppState::new(client_state());

    app.handle_key(key(KeyCode::Char(',')));
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Tab));

    render_app(&app);
}

#[test]
fn selected_request_stays_visible_beyond_first_screen() {
    let mut state = client_state();
    state.requests = (0..100)
        .map(|index| crate::test_support::request(&format!("item-{index:03}"), "default"))
        .collect();
    state.selected_request_id = Some("item-000".into());
    let screen = render_app(&AppState::new(state));
    assert!(
        screen.contains("> item-000"),
        "selected list row must be in viewport"
    );
}

#[test]
fn detail_end_key_reaches_last_line_of_long_request() {
    let mut state = client_state();
    state.requests[0].body_markdown = (0..100)
        .map(|index| format!("Line {index:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = AppState::new(state);
    let first = render_app(&app);
    assert!(!first.contains("Line 099"));
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::End));
    assert!(render_app(&app).contains("Line 099"));
}

fn render_app(app: &AppState) -> String {
    let backend = TestBackend::new(100, 32);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");

    terminal
        .draw(|frame| render(frame, app))
        .expect("render should complete without panicking");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
