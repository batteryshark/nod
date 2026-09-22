use nod_client_core::models::{Channel, Request, RequestStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{AppState, EnrollmentField, EnrollmentForm, Focus, Modal},
    domain,
};

use super::{
    format::{form_line, option_key_hint, request_status_label, selected_marker, status_label},
    layout::{centered_inner, centered_rect, focused_block},
};

pub(super) fn render_main(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(28),
            Constraint::Percentage(36),
            Constraint::Percentage(64),
        ])
        .split(vertical[0]);

    render_sidebar(frame, columns[0], app);
    render_request_list(frame, columns[1], app);
    render_detail(frame, columns[2], app);
    render_status(frame, vertical[1], app);
}

pub(super) fn render_enrollment(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let block = Block::default().title("Nod").borders(Borders::ALL);
    frame.render_widget(block, centered_rect(56, 12, area));
    if let Some(Modal::Enrollment(form)) = app.modal() {
        render_enrollment_form(
            frame,
            centered_inner(56, 12, area),
            form,
            app.error().or_else(|| app.running()),
        );
    }
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(area);
    render_servers(frame, sections[0], app);
    render_channels(frame, sections[1], app);
}

fn render_servers(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let selected_id = domain::selected_server_id(app.client_state());
    let items: Vec<ListItem<'_>> = app
        .client_state()
        .servers
        .iter()
        .map(|server| {
            let marker = selected_marker(selected_id == Some(server.id.as_str()));
            ListItem::new(Line::from(format!("{marker}{}", server.name)))
        })
        .collect();

    let selected = app
        .client_state()
        .servers
        .iter()
        .position(|server| Some(server.id.as_str()) == selected_id);
    frame.render_stateful_widget(
        List::new(items)
            .block(focused_block("Servers", app.focus() == Focus::Servers))
            .style(Style::default().fg(Color::White)),
        area,
        &mut ListState::default().with_selected(selected),
    );
}

fn render_channels(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let selected_id =
        domain::selected_channel(app.client_state()).map(|channel| channel.id.as_str());
    let mut items: Vec<ListItem<'_>> = domain::subscribed_channels(app.client_state())
        .iter()
        .map(|channel| channel_item(channel, selected_id, app))
        .collect();

    items.insert(
        0,
        ListItem::new(format!(
            "{}All channels (0)",
            selected_marker(selected_id.is_none())
        )),
    );
    let selected = selected_id
        .and_then(|id| {
            domain::subscribed_channels(app.client_state())
                .iter()
                .position(|channel| channel.id == id)
        })
        .map(|index| index + 1)
        .unwrap_or(0);
    frame.render_stateful_widget(
        List::new(items).block(focused_block("Channels", app.focus() == Focus::Channels)),
        area,
        &mut ListState::default().with_selected(Some(selected)),
    );
}

fn channel_item<'a>(channel: &Channel, selected_id: Option<&str>, app: &AppState) -> ListItem<'a> {
    let marker = selected_marker(selected_id == Some(channel.id.as_str()));
    let count = domain::pending_count_for(channel, app.client_state());
    ListItem::new(Line::from(format!(
        "{marker}{} {} ({count})",
        channel.emoji, channel.name
    )))
}

fn render_request_list(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let selected_id = app.selected_request().map(|request| request.id.as_str());
    let items: Vec<ListItem<'_>> = app
        .visible_requests()
        .into_iter()
        .map(|request| request_item(request, selected_id))
        .collect();
    let title = if app.filter().is_empty() {
        format!("Requests{}", app.history_hint())
    } else {
        format!("Requests /{}{}", app.filter(), app.history_hint())
    };

    let empty = if items.is_empty() {
        vec![ListItem::new("No requests")]
    } else {
        items
    };
    let selected = app
        .visible_requests()
        .iter()
        .position(|request| Some(request.id.as_str()) == selected_id);
    frame.render_stateful_widget(
        List::new(empty).block(focused_block(&title, app.focus() == Focus::Requests)),
        area,
        &mut ListState::default().with_selected(selected),
    );
}

fn request_item<'a>(request: &Request, selected_id: Option<&str>) -> ListItem<'a> {
    let marker = selected_marker(selected_id == Some(request.id.as_str()));
    let status = request_status_label(&request.status);
    let summary = if request.summary.is_empty() {
        request.body_markdown.as_str()
    } else {
        request.summary.as_str()
    };
    ListItem::new(vec![
        Line::from(format!("{marker}{} [{status}]", request.title)),
        Line::from(Span::styled(
            format!("  {summary}"),
            Style::default().fg(Color::DarkGray),
        )),
    ])
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let Some(request) = app.selected_request() else {
        frame.render_widget(
            Paragraph::new("Select a request")
                .block(focused_block("Detail", app.focus() == Focus::Detail)),
            area,
        );
        return;
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                &request.title,
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", status_label(request))),
        ]),
        Line::from(request.summary.clone()),
        Line::from(""),
    ];
    if !request.body_markdown.is_empty() {
        lines.extend(
            request
                .body_markdown
                .lines()
                .map(|line| Line::from(line.to_string())),
        );
        lines.push(Line::from(""));
    }
    for field in &request.fields {
        lines.push(Line::from(format!("{}: {}", field.label, field.value)));
    }
    if !request.links.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Links"));
        for link in &request.links {
            lines.push(Line::from(format!("{} - {}", link.label, link.url)));
        }
    }
    if let Some(expires_at) = request.expires_at {
        lines.push(Line::from(format!("Expires: {expires_at}")));
    }
    if let Some(decision) = &request.decision {
        lines.push(Line::from(format!(
            "Decision: {} · {}",
            decision.option_label, decision.resolved_at
        )));
        if let Some(actor) = &decision.actor_user_id {
            lines.push(Line::from(format!("By: {actor}")));
        }
        if let Some(notes) = &decision.text {
            lines.push(Line::from(format!("Notes: {notes}")));
        }
    }
    lines.push(Line::from(format!(
        "Request: {} · Channel: {}",
        request.id, request.channel_id
    )));
    if request.status == RequestStatus::Pending {
        lines.push(Line::from(""));
        lines.push(Line::from("Options"));
        if request.options.is_empty() {
            lines.push(Line::from("d dismiss"));
        } else {
            for option in &request.options {
                lines.push(Line::from(format!(
                    "{}  {}",
                    option_key_hint(option.kind.as_str()),
                    option.label
                )));
            }
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let height = area.height.saturating_sub(2);
    let line_count = paragraph.line_count(area.width.saturating_sub(2));
    let maximum = line_count
        .saturating_sub(usize::from(height))
        .min(usize::from(u16::MAX)) as u16;
    let scroll = app.clamp_detail_scroll(maximum);
    let title = if maximum > 0 {
        format!(
            "Detail · {}/{} · PgUp/PgDn",
            scroll.saturating_add(1),
            maximum.saturating_add(1)
        )
    } else {
        "Detail".into()
    };
    frame.render_widget(
        paragraph
            .scroll((scroll, 0))
            .block(focused_block(&title, app.focus() == Focus::Detail)),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let pending = domain::total_pending_count(app.client_state());
    let sync = match app.client_state().sync_phase {
        nod_client_core::models::SyncPhase::Current => "sync:current",
        nod_client_core::models::SyncPhase::Connecting => "sync:connecting",
        nod_client_core::models::SyncPhase::Reconciling => "sync:reconciling",
        nod_client_core::models::SyncPhase::Revoked => "sync:revoked",
        nod_client_core::models::SyncPhase::Offline => "sync:offline",
    };
    let alerts = if app.alerts().muted() {
        "alerts:muted"
    } else {
        "alerts:on"
    };
    let message = app
        .error()
        .or_else(|| app.running())
        .or_else(|| app.alerts().message())
        .unwrap_or_else(|| app.status());
    let style = if app.error().is_some() {
        Style::default().fg(Color::Red)
    } else if app.alerts().flashing() {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    let text = format!("pending:{pending}  {sync}  {alerts}  {message}");
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn render_enrollment_form(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &EnrollmentForm,
    error: Option<&str>,
) {
    let lines = vec![
        form_line(
            "Server",
            form.base_url().value(),
            form.active_field() == EnrollmentField::Server,
        ),
        form_line(
            "Device",
            form.device_name().value(),
            form.active_field() == EnrollmentField::Device,
        ),
        form_line(
            "Code",
            form.code().value(),
            form.active_field() == EnrollmentField::Code,
        ),
        form_line(
            "Sound",
            form.selected_sound(),
            form.active_field() == EnrollmentField::Sound,
        ),
        Line::from(""),
        Line::from(error.unwrap_or("Enter to enroll. Tab moves fields.")),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
