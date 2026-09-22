use nod_client_core::models::{DevicePlatform, Request, RequestStatus, UserDevice};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

pub(super) fn selected_marker(selected: bool) -> &'static str {
    if selected {
        "> "
    } else {
        "  "
    }
}

pub(super) fn checkbox(checked: bool) -> &'static str {
    if checked {
        "[x]"
    } else {
        "[ ]"
    }
}

pub(super) fn form_line<'a>(label: &'a str, value: &'a str, active: bool) -> Line<'a> {
    Line::from(format!("{}{label}: {value}", selected_marker(active)))
}

pub(super) fn tab_label(label: &'static str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            format!(" {label} "),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )
    } else {
        Span::raw(format!(" {label} "))
    }
}

pub(super) fn status_label(request: &Request) -> &'static str {
    request_status_label(&request.status)
}

pub(super) fn request_status_label(status: &RequestStatus) -> &'static str {
    match status {
        RequestStatus::Pending => "pending",
        RequestStatus::Resolved => "resolved",
        RequestStatus::Expired => "expired",
        RequestStatus::Cancelled => "cancelled",
    }
}

pub(super) fn option_key_hint(kind: &str) -> &'static str {
    match kind {
        "approve" | "approve_with_text" => "a",
        "reject" | "reject_with_text" => "r",
        "dismiss" => "d",
        _ => "o",
    }
}

pub(super) fn platform_label(device: &UserDevice) -> &'static str {
    match device.platform {
        DevicePlatform::Ios => "ios",
        DevicePlatform::Macos => "macos",
        DevicePlatform::Watchos => "watchos",
        DevicePlatform::Windows => "windows",
        DevicePlatform::Linux => "linux",
        DevicePlatform::Unknown => "unknown",
    }
}
