use nod_client_core::models::Request;

/// The toast body only. There are deliberately no `<actions>`: a decision is
/// made in the app window over the request as rendered there, never from a
/// notification button. Clicking the toast launches the app on the request.
pub(super) fn windows_toast_xml(request: &Request, sound: &str) -> String {
    let preview = nod_proto::notification_preview(request);
    format!(
        "<toast launch=\"open\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>{}</toast>",
        xml_escape(&preview.title),
        xml_escape(&preview.body),
        if matches!(sound, "silent" | "none") { "<audio silent=\"true\"/>" } else { "" }
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// Windows notification tags/groups must fit the oldest supported 16-character limit.
pub(super) fn notification_tag(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))[..16].to_string()
}
