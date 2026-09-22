use nod_client_core::models::{
    Channel, ClientState, OptionKind, Request, RequestOption, RequestStatus,
};

pub fn total_pending_count(state: &ClientState) -> usize {
    state.pending_counts_by_channel.values().sum()
}

pub fn pending_count_for(channel: &Channel, state: &ClientState) -> usize {
    state
        .pending_counts_by_channel
        .get(&channel.id)
        .copied()
        .unwrap_or_default()
}

pub fn subscribed_channels(state: &ClientState) -> Vec<&Channel> {
    state
        .channels
        .iter()
        .filter(|channel| channel.subscribed)
        .collect()
}

pub fn ordered_requests(requests: &[Request]) -> Vec<&Request> {
    ordered_request_refs(requests.iter())
}

pub fn ordered_request_refs<'a>(requests: impl Iterator<Item = &'a Request>) -> Vec<&'a Request> {
    let mut ordered: Vec<_> = requests.collect();
    ordered.sort_by(|left, right| {
        status_rank(&left.status)
            .cmp(&status_rank(&right.status))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| right.id.cmp(&left.id))
    });
    ordered
}

pub fn selected_channel(state: &ClientState) -> Option<&Channel> {
    state
        .selected_channel_id
        .as_deref()
        .and_then(|id| state.channels.iter().find(|channel| channel.id == id))
}

pub fn selected_request(state: &ClientState) -> Option<&Request> {
    state
        .selected_request_id
        .as_deref()
        .and_then(|id| state.requests.iter().find(|request| request.id == id))
        .or_else(|| ordered_requests(&state.requests).into_iter().next())
}

pub fn selected_server_id(state: &ClientState) -> Option<&str> {
    state
        .selected_server_id
        .as_deref()
        .or_else(|| state.servers.first().map(|server| server.id.as_str()))
}

pub fn option_requires_text(option: &RequestOption) -> bool {
    option.requires_text
        || matches!(
            option.kind,
            OptionKind::ApproveWithText | OptionKind::RejectWithText
        )
}

pub fn option_for_kind<'a>(request: &'a Request, kind: OptionKind) -> Option<OptionChoice<'a>> {
    request
        .options
        .iter()
        .find(|option| option.kind == kind)
        // The detail pane hints the same key for an option's with-text
        // variant, so the key must reach it too (the text editor opens via
        // requires_text). Exact kind wins when a request offers both.
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| Some(&option.kind) == with_text_variant(&kind).as_ref())
        })
        .map(OptionChoice::from_option)
        .or_else(|| default_dismiss_option(request, &kind))
}

fn with_text_variant(kind: &OptionKind) -> Option<OptionKind> {
    match kind {
        OptionKind::Approve => Some(OptionKind::ApproveWithText),
        OptionKind::Reject => Some(OptionKind::RejectWithText),
        _ => None,
    }
}

pub fn submittable_options(request: &Request) -> Vec<RequestOption> {
    if request.options.is_empty() {
        vec![RequestOption {
            id: "dismiss".into(),
            label: "Dismiss".into(),
            kind: OptionKind::Dismiss,
            style: "default".into(),
            requires_text: false,
            text_placeholder: None,
            destructive: false,
            foreground: false,
        }]
    } else {
        request.options.clone()
    }
}

pub fn first_text_option(request: &Request) -> Option<OptionChoice<'_>> {
    request
        .options
        .iter()
        .find(|option| option_requires_text(option))
        .map(OptionChoice::from_option)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionChoice<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub placeholder: Option<&'a str>,
    pub requires_text: bool,
}

impl<'a> OptionChoice<'a> {
    pub(crate) fn from_option(option: &'a RequestOption) -> Self {
        Self {
            id: &option.id,
            label: &option.label,
            placeholder: option.text_placeholder.as_deref(),
            requires_text: option_requires_text(option),
        }
    }
}

fn default_dismiss_option<'a>(request: &'a Request, kind: &OptionKind) -> Option<OptionChoice<'a>> {
    if *kind != OptionKind::Dismiss || !request.options.is_empty() {
        return None;
    }

    // The core signer recognizes this implicit option for optionless requests,
    // so the TUI can offer a consistent dismiss key without server metadata.
    Some(OptionChoice {
        id: "dismiss",
        label: "Dismiss",
        placeholder: None,
        requires_text: false,
    })
}

fn status_rank(status: &RequestStatus) -> u8 {
    match status {
        RequestStatus::Pending => 0,
        RequestStatus::Resolved => 1,
        RequestStatus::Expired => 1,
        RequestStatus::Cancelled => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{client_state, request_with_status};
    use nod_client_core::models::RequestStatus;

    #[test]
    fn option_key_reaches_the_with_text_variant() {
        use nod_client_core::models::{OptionKind, RequestOption};

        let mut request = request_with_status("deploy", "default", RequestStatus::Pending);
        request.options.push(RequestOption {
            id: "approve".to_string(),
            label: "Approve".to_string(),
            kind: OptionKind::ApproveWithText,
            style: "primary".to_string(),
            requires_text: true,
            text_placeholder: None,
            destructive: false,
            foreground: false,
        });

        let choice = option_for_kind(&request, OptionKind::Approve)
            .expect("the approve key should reach the approve_with_text option");
        assert_eq!(choice.id, "approve");
        assert!(choice.requires_text);
    }

    #[test]
    fn ordered_requests_keep_pending_before_handled() {
        let resolved = request_with_status("resolved", "default", RequestStatus::Resolved);
        let pending = request_with_status("pending", "default", RequestStatus::Pending);
        let requests = vec![resolved, pending];

        let ordered = ordered_requests(&requests);

        assert_eq!(ordered[0].id, "pending");
        assert_eq!(ordered[1].id, "resolved");
    }

    #[test]
    fn selected_request_falls_back_to_first_ordered_request() {
        let mut state = client_state();
        state.requests = vec![
            request_with_status("handled", "default", RequestStatus::Resolved),
            request_with_status("pending", "default", RequestStatus::Pending),
        ];

        let selected = selected_request(&state).map(|request| request.id.as_str());

        assert_eq!(selected, Some("pending"));
    }
}
