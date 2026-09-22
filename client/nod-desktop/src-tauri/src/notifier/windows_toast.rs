use nod_client_core::models::Request;

use super::options::desktop_notification_options;

pub(super) fn windows_toast_xml(request: &Request, sound: &str) -> String {
    let preview = nod_proto::notification_preview(request);
    let options = desktop_notification_options("", request)
        .into_iter()
        .map(|option| {
            format!(
                "<action content=\"{}\" arguments=\"action:{}\" activationType=\"foreground\"/>",
                xml_escape(&option.label),
                xml_escape(&option.id)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        "<toast launch=\"open\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual><actions>{}</actions>{}</toast>",
        xml_escape(&preview.title),
        xml_escape(&preview.body),
        options,
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
#[cfg(any(target_os = "windows", test))]
pub(super) fn notification_tag(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))[..16].to_string()
}
