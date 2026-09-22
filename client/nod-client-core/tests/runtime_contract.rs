use nod_client_core::{models::ClientState, NodClientMessage};
use serde_json::Value;

// Shared with the desktop tests: serde renames, enum tags, defaults, and
// omitted/null fields must keep their wire meaning across the language boundary.
#[test]
fn serialized_runtime_messages_match_cross_language_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/runtime-messages.json")).unwrap();
    let state: ClientState =
        serde_json::from_value(fixture["messages"][0]["payload"].clone()).unwrap();
    let server_id = state.selected_server_id.clone().unwrap();
    let request = state.requests[0].clone();
    let request_id = request.id.clone();
    let messages = vec![
        NodClientMessage::State(Box::new(state)),
        NodClientMessage::NotificationCandidate {
            server_id: server_id.clone(),
            request: Box::new(request),
        },
        NodClientMessage::NotificationRemoved {
            server_id,
            request_id,
        },
    ];
    assert_eq!(serde_json::to_value(messages).unwrap(), fixture["messages"]);
}
