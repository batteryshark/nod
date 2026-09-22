use nod_client_core::models::UserDevice;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{
    AppState, Modal, OptionTextForm, RenameDeviceForm, SettingsState, SettingsTab, SOUND_OPTIONS,
};

use super::{
    format::{checkbox, platform_label, selected_marker, tab_label},
    layout::{centered_rect, render_box},
};

pub(super) fn render_modal(frame: &mut Frame<'_>, area: Rect, app: &AppState, modal: &Modal) {
    match modal {
        Modal::Enrollment(_) => {
            frame.render_widget(Clear, centered_rect(56, 12, area));
            super::panes::render_enrollment(frame, area, app);
        }
        Modal::OptionText(form) => render_option_text_modal(frame, area, form, app),
        Modal::Settings(settings) => render_settings_modal(frame, area, app, settings),
        Modal::RenameDevice(form) => render_rename_modal(frame, area, form),
        Modal::Filter(input) => render_text_modal(frame, area, "Filter", input.value()),
        Modal::Help => render_help_modal(frame, area),
        Modal::Actions { request, selected } => {
            let options = crate::domain::submittable_options(request);
            let items: Vec<_> = options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    ListItem::new(format!(
                        "{}{}{}",
                        selected_marker(index == *selected),
                        option.label,
                        if crate::domain::option_requires_text(option) {
                            " (notes)"
                        } else {
                            ""
                        }
                    ))
                })
                .collect();
            let modal_area = centered_rect(64, 14, area);
            frame.render_widget(Clear, modal_area);
            frame.render_stateful_widget(
                List::new(items).block(
                    Block::default()
                        .title("Actions · Enter selects · Esc cancels")
                        .borders(Borders::ALL),
                ),
                modal_area,
                &mut ListState::default().with_selected(Some(*selected)),
            );
        }
        Modal::Confirm(action) => render_box(
            frame,
            area,
            "Confirm",
            vec![
                Line::from(action.prompt()),
                Line::from(""),
                Line::from(
                    app.error()
                        .or_else(|| app.running())
                        .unwrap_or("Enter confirms. Esc cancels."),
                ),
            ],
            70,
            8,
        ),
    }
}

fn render_option_text_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &OptionTextForm,
    app: &AppState,
) {
    let title = format!("{} notes", form.label());
    let hint = form.placeholder().unwrap_or("Text");
    let body = vec![
        Line::from(hint.to_string()),
        Line::from(form.input().value().to_string()),
        Line::from(""),
        Line::from(
            app.error()
                .or_else(|| app.running())
                .unwrap_or("Enter submits. Esc cancels."),
        ),
    ];
    render_box(frame, area, &title, body, 64, 9);
}

fn render_rename_modal(frame: &mut Frame<'_>, area: Rect, form: &RenameDeviceForm) {
    let body = vec![
        Line::from("New name"),
        Line::from(form.input().value().to_string()),
        Line::from(""),
        Line::from("Enter renames. Esc cancels."),
    ];
    render_box(frame, area, "Rename Device", body, 52, 8);
}

fn render_text_modal(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str) {
    let body = vec![
        Line::from(value.to_string()),
        Line::from(""),
        Line::from("Enter applies. Esc cancels."),
    ];
    render_box(frame, area, title, body, 52, 7);
}

fn render_settings_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    settings: &SettingsState,
) {
    let modal_area = centered_rect(78, 22, area);
    frame.render_widget(Clear, modal_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(modal_area);
    frame.render_widget(
        Block::default().title("Settings").borders(Borders::ALL),
        modal_area,
    );
    render_settings_tabs(frame, chunks[0], settings);
    render_settings_body(frame, chunks[1], app, settings);
    frame.render_widget(
        Paragraph::new("Tab tabs. Space toggles. r rename. x revoke. f forget server. Esc closes.")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_settings_tabs(frame: &mut Frame<'_>, area: Rect, settings: &SettingsState) {
    let labels = [
        tab_label("Channels", settings.tab() == SettingsTab::Channels),
        tab_label("Sound", settings.tab() == SettingsTab::Sound),
        tab_label("Devices", settings.tab() == SettingsTab::Devices),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(labels.to_vec())).alignment(Alignment::Center),
        area,
    );
}

fn render_settings_body(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    settings: &SettingsState,
) {
    match settings.tab() {
        SettingsTab::Channels => render_settings_channels(frame, area, app, settings),
        SettingsTab::Sound => render_settings_sound(frame, area, app, settings),
        SettingsTab::Devices => render_settings_devices(frame, area, app.devices(), settings),
    }
}

fn render_settings_channels(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    settings: &SettingsState,
) {
    let items: Vec<_> = app
        .client_state()
        .channels
        .iter()
        .enumerate()
        .map(|(index, channel)| {
            let marker = selected_marker(settings.selected_index() == index);
            let checked = checkbox(channel.subscribed);
            ListItem::new(format!(
                "{marker}{checked} {} {}",
                channel.emoji, channel.name
            ))
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items),
        area,
        &mut ListState::default().with_selected(Some(settings.selected_index())),
    );
}

fn render_settings_sound(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    settings: &SettingsState,
) {
    let current = app.client_state().notification_sound.as_str();
    let items: Vec<_> = SOUND_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, sound)| {
            let marker = selected_marker(settings.selected_index() == index);
            let checked = checkbox(current == *sound);
            ListItem::new(format!("{marker}{checked} {sound}"))
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items),
        area,
        &mut ListState::default().with_selected(Some(settings.selected_index())),
    );
}

fn render_settings_devices(
    frame: &mut Frame<'_>,
    area: Rect,
    devices: &[UserDevice],
    settings: &SettingsState,
) {
    let items: Vec<_> = devices
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let marker = selected_marker(settings.selected_index() == index);
            let current = if device.is_current { " current" } else { "" };
            ListItem::new(format!(
                "{marker}{}  {}{}",
                device.name,
                platform_label(device),
                current
            ))
        })
        .collect();
    let empty = if items.is_empty() {
        vec![ListItem::new("No devices loaded")]
    } else {
        items
    };
    frame.render_stateful_widget(
        List::new(empty),
        area,
        &mut ListState::default().with_selected(Some(settings.selected_index())),
    );
}

fn render_help_modal(frame: &mut Frame<'_>, area: Rect) {
    let body = vec![
        Line::from("j/k or arrows move"),
        Line::from("Tab changes focus"),
        Line::from("Enter opens detail or submits form"),
        Line::from("a approve, r reject, d dismiss, n notes, o all actions"),
        Line::from("PgUp/PgDn scroll; Home/End in detail; 0 all channels"),
        Line::from("e enroll another server; H history, ] older"),
        Line::from("c clear channel, R refresh, / filter"),
        Line::from("s server focus, , settings, m mute alerts"),
        Line::from("Esc closes forms; q quits outside text fields"),
    ];
    render_box(frame, area, "Help", body, 68, 14);
}
