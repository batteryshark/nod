use nod_client_core::models::Request;
use tokio::sync::mpsc;
use windows::{
    core::{IInspectable, HSTRING},
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
};

use crate::notifier::{
    windows_toast::{notification_tag, windows_toast_xml},
    NotificationActivation, NotificationContext,
};

const TOAST_APP_ID: &str = "com.stonefishlabs.nod.desktop";

/// Windows only displays toasts for a registered AppUserModelID. Installers
/// register one via a Start-menu shortcut; a bare unzipped exe has to use the
/// per-user registry registration instead, refreshed on every launch.
pub(crate) fn register_toast_app_id(app: &tauri::App) -> anyhow::Result<()> {
    use tauri::Manager;

    let data_dir = app.path().app_local_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let icon_path = data_dir.join("toast-icon.png");
    std::fs::write(&icon_path, include_bytes!("../../../icons/128x128.png"))?;

    let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{TOAST_APP_ID}"))?;
    key.set_value("DisplayName", &"Nod")?;
    key.set_value("IconUri", &icon_path.to_string_lossy().to_string())?;
    Ok(())
}

pub(crate) async fn show_notification(
    request: &Request,
    context: &NotificationContext,
    activations: mpsc::Sender<NotificationActivation>,
) -> anyhow::Result<()> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(windows_toast_xml(request, &context.sound)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.SetTag(&HSTRING::from(notification_tag(&request.id)))?;
    toast.SetGroup(&HSTRING::from(notification_tag(&context.server_id)))?;
    let request_id = request.id.clone();
    let server_id = context.server_id.clone();
    // The handler's generics pin the closure argument types — the activation
    // callback receives (&Option<ToastNotification>, &Option<IInspectable>).
    // The toast has no actions, so any activation just opens the request;
    // the activation arguments are deliberately not consulted.
    let handler = TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
        let _ = activations.blocking_send(NotificationActivation {
            server_id: server_id.clone(),
            request_id: request_id.clone(),
        });
        Ok(())
    });
    toast.Activated(&handler)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(TOAST_APP_ID))?;
    notifier.Show(&toast)?;
    Ok(())
}

pub(crate) async fn remove_notification(server_id: &str, request_id: &str) -> anyhow::Result<()> {
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(
        &HSTRING::from(notification_tag(request_id)),
        &HSTRING::from(notification_tag(server_id)),
        &HSTRING::from(TOAST_APP_ID),
    )?;
    Ok(())
}
