#[cfg(any(target_os = "windows", test))]
mod badge;
mod commands;
mod desktop_state;
mod external_url;
mod media;
mod notifier;
mod preferences;
mod runtime_messages;
mod tray;
mod window;

use std::{error::Error, sync::Arc};

use nod_client_core::{NodClientMessage, NodClientRuntime};
use tauri::{App, AppHandle, Manager};
use tokio::sync::{mpsc, Mutex};

use crate::{
    desktop_state::DesktopState,
    notifier::DesktopNotifier,
    preferences::PreferenceStore,
    runtime_messages::{emit_transient_error, forward_runtime_messages},
    tray::install_tray,
    window::focus_main_window,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::{notifier::NotificationActivation, runtime_messages::handle_notification_activations};

const RUNTIME_MESSAGE_BUFFER: usize = 128;
#[cfg(any(target_os = "linux", target_os = "windows"))]
const NOTIFICATION_ACTIVATION_BUFFER: usize = 32;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(setup_desktop)
        .invoke_handler(tauri::generate_handler![
            commands::state,
            commands::enroll,
            commands::refresh,
            commands::select_server,
            commands::forget_server,
            commands::select_channel,
            commands::select_all_channels,
            commands::select_request,
            commands::open_request,
            commands::query_history,
            commands::submit_option,
            commands::submit_request_option,
            commands::clear_channel,
            commands::set_subscription,
            commands::set_notification_preference,
            commands::list_devices,
            commands::rename_device,
            commands::revoke_device,
            commands::desktop_preferences,
            commands::set_desktop_preferences,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::test_notification,
            media::request_image,
            commands::open_external_url
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Nod desktop");
}

fn setup_desktop(app: &mut App) -> Result<(), Box<dyn Error>> {
    install_tray(app)?;

    let app_handle = app.handle().clone();
    #[cfg(target_os = "windows")]
    if let Err(error) = notifier::register_toast_app_id(app) {
        emit_transient_error(&app_handle, "register toast app id", error);
    }
    let (message_tx, message_rx) = mpsc::channel::<NodClientMessage>(RUNTIME_MESSAGE_BUFFER);
    let runtime = tauri::async_runtime::block_on(NodClientRuntime::new(message_tx))?;
    let runtime = Arc::new(Mutex::new(runtime));
    let preferences = Arc::new(Mutex::new(PreferenceStore::open(
        app.path().app_config_dir()?,
    )?));
    let notifier = desktop_notifier(app_handle.clone(), runtime.clone(), preferences.clone());

    app.manage(DesktopState::new(
        runtime.clone(),
        preferences,
        notifier.clone(),
    ));

    tauri::async_runtime::spawn(forward_runtime_messages(
        app_handle.clone(),
        notifier,
        message_rx,
    ));
    tauri::async_runtime::spawn(async move {
        let mut runtime = runtime.lock().await;
        runtime.emit_ready().await;
        runtime.emit_state().await;
        // Already-registered devices reconnect on launch, matching the TUI's
        // startup commands; without this the app only syncs over HTTP.
        if runtime.state().await.is_registered {
            if let Err(error) = runtime.refresh().await {
                emit_transient_error(&app_handle, "refresh on launch", error);
            }
            if let Err(error) = runtime.connect_sync().await {
                emit_transient_error(&app_handle, "connect sync", error);
            }
        }
    });

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn desktop_notifier(
    app_handle: AppHandle,
    runtime: Arc<Mutex<NodClientRuntime>>,
    preferences: Arc<Mutex<PreferenceStore>>,
) -> DesktopNotifier {
    let (activation_tx, activation_rx) =
        mpsc::channel::<NotificationActivation>(NOTIFICATION_ACTIVATION_BUFFER);
    tauri::async_runtime::spawn(handle_notification_activations(
        app_handle,
        runtime,
        activation_rx,
    ));
    DesktopNotifier::new(activation_tx, preferences)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn desktop_notifier(
    _app_handle: AppHandle,
    _runtime: Arc<Mutex<NodClientRuntime>>,
    preferences: Arc<Mutex<PreferenceStore>>,
) -> DesktopNotifier {
    DesktopNotifier::new(preferences)
}
