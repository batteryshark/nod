use nod_client_core::models::Request;
use notify_rust::{ActionResponse, Hint, Notification};
use std::sync::{Mutex, OnceLock};

use super::notification_registry::NotificationRegistry;
use tokio::sync::mpsc;

use crate::notifier::{
    options::desktop_notification_options, NotificationActivation, NotificationContext,
};

pub(crate) async fn show_notification(
    request: &Request,
    context: &NotificationContext,
    activations: mpsc::Sender<NotificationActivation>,
) -> anyhow::Result<()> {
    let preview = nod_proto::notification_preview(request);
    let options = desktop_notification_options(&context.server_id, request);
    let key = format!("{}:{}", context.server_id, request.id);
    let mut notification = Notification::new();
    notification
        .summary(&preview.title)
        .body(&preview.body)
        .appname("Nod")
        .hint(Hint::Category("email".to_string()));
    if let Some(id) = notification_ids()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
    {
        notification.id(id.id);
    }
    if matches!(context.sound.as_str(), "silent" | "none") {
        notification.hint(Hint::SuppressSound(true));
    }
    notification.action("default", "Open Nod");
    for option in &options {
        notification.action(&format!("action:{}", option.id), &option.label);
    }
    let handle = notification.show_async().await?;
    let identity = notification_ids()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .replace(key.clone(), handle.id());
    let request_id = request.id.clone();
    let server_id = context.server_id.clone();
    // Replacement can reuse the OS id. Only the newest waiter may activate or
    // remove that request's entry; an older close callback must not erase it.
    tauri::async_runtime::spawn(async move {
        let mut activation = None;
        handle
            .wait_for_action_async(|response| {
                let ActionResponse::Custom(option_id) = response else {
                    return;
                };
                let is_current = notification_ids()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get(&key)
                    == Some(identity);
                if !is_current {
                    return;
                }
                activation = Some(
                    options
                        .iter()
                        .find(|option| {
                            Some(option.id.as_str()) == option_id.strip_prefix("action:")
                        })
                        .map(|option| option.activation.clone())
                        .unwrap_or(NotificationActivation::Open {
                            server_id: server_id.clone(),
                            request_id: Some(request_id.clone()),
                        }),
                );
            })
            .await;
        notification_ids()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove_if_current(&key, identity);
        if let Some(activation) = activation {
            let _ = activations.send(activation).await;
        }
    });
    Ok(())
}

pub(crate) async fn remove_notification(server_id: &str, request_id: &str) -> anyhow::Result<()> {
    let key = format!("{server_id}:{request_id}");
    let id = notification_ids()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key);
    let Some(id) = id else { return Ok(()) };
    let connection = zbus::Connection::session().await?;
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &id.id,
        )
        .await?;
    notification_ids()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove_if_current(&key, id);
    Ok(())
}

fn notification_ids() -> &'static Mutex<NotificationRegistry> {
    static IDS: OnceLock<Mutex<NotificationRegistry>> = OnceLock::new();
    IDS.get_or_init(|| Mutex::new(NotificationRegistry::default()))
}
