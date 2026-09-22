use nod_client_core::models::ClientState;
use nod_client_core::{
    ChannelParams, EnrollParams, NotificationPreferenceParams, RenameDeviceParams,
    RevokeDeviceParams, SelectRequestParams, SelectServerParams, SetSubscriptionParams,
    SubmitOptionParams,
};
use tauri::State;

use crate::{desktop_state::DesktopState, external_url::open_url};

#[tauri::command]
pub(crate) async fn state(state: State<'_, DesktopState>) -> Result<ClientState, String> {
    Ok(state.runtime.lock().await.state().await)
}

#[tauri::command]
pub(crate) async fn enroll(
    state: State<'_, DesktopState>,
    app: tauri::AppHandle,
    params: EnrollParams,
) -> Result<ClientState, String> {
    let mut runtime = state.runtime.lock().await;
    command_result(runtime.enroll(params).await)?;
    // Enrollment returns a state snapshot; starting sync here matches the
    // startup path that already-registered devices use.
    if let Err(error) = runtime.connect_sync().await {
        crate::runtime_messages::emit_transient_error(
            &app,
            "Registered; could not start sync",
            error,
        );
    }
    Ok(runtime.state().await)
}

#[tauri::command]
pub(crate) async fn refresh(state: State<'_, DesktopState>) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.refresh().await)
}

#[tauri::command]
pub(crate) async fn select_server(
    state: State<'_, DesktopState>,
    params: SelectServerParams,
) -> Result<ClientState, String> {
    command_result(
        state
            .runtime
            .lock()
            .await
            .select_server(params.server_id)
            .await,
    )
}

#[tauri::command]
pub(crate) async fn forget_server(
    state: State<'_, DesktopState>,
    params: SelectServerParams,
) -> Result<ClientState, String> {
    command_result(
        state
            .runtime
            .lock()
            .await
            .forget_server(&params.server_id)
            .await,
    )
}

#[tauri::command]
pub(crate) async fn select_channel(
    state: State<'_, DesktopState>,
    params: ChannelParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.select_channel(params).await)
}

#[tauri::command]
pub(crate) async fn select_request(
    state: State<'_, DesktopState>,
    params: SelectRequestParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.select_request(params).await)
}

#[tauri::command]
pub(crate) async fn submit_option(
    state: State<'_, DesktopState>,
    params: SubmitOptionParams,
) -> Result<nod_client_core::models::Request, String> {
    command_result(state.runtime.lock().await.submit_option(params).await)
}

#[tauri::command]
pub(crate) async fn clear_channel(
    state: State<'_, DesktopState>,
    params: ChannelParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.clear_channel(params).await)
}

#[tauri::command]
pub(crate) async fn set_subscription(
    state: State<'_, DesktopState>,
    params: SetSubscriptionParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.set_subscription(params).await)
}

#[tauri::command]
pub(crate) async fn set_notification_preference(
    state: State<'_, DesktopState>,
    params: NotificationPreferenceParams,
) -> Result<ClientState, String> {
    command_result(
        state
            .runtime
            .lock()
            .await
            .set_notification_preference(&params.notification_sound)
            .await,
    )
}

#[tauri::command]
pub(crate) async fn list_devices(
    state: State<'_, DesktopState>,
) -> Result<Vec<nod_client_core::models::UserDevice>, String> {
    command_result(state.runtime.lock().await.list_devices().await)
}

#[tauri::command]
pub(crate) async fn rename_device(
    state: State<'_, DesktopState>,
    params: RenameDeviceParams,
) -> Result<nod_client_core::models::UserDevice, String> {
    command_result(state.runtime.lock().await.rename_device(params).await)
}

#[tauri::command]
pub(crate) async fn revoke_device(
    state: State<'_, DesktopState>,
    params: RevokeDeviceParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.revoke_device(params).await)
}

#[tauri::command]
pub(crate) fn open_external_url(url: String) -> Result<(), String> {
    open_url(&url).map_err(|error| error.to_string())
}

fn command_result<T>(result: anyhow::Result<T>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn desktop_preferences(
    state: State<'_, DesktopState>,
) -> Result<crate::preferences::DesktopPreferences, String> {
    Ok(state.preferences.lock().await.get())
}

#[tauri::command]
pub(crate) async fn set_desktop_preferences(
    state: State<'_, DesktopState>,
    preferences: crate::preferences::DesktopPreferences,
) -> Result<crate::preferences::DesktopPreferences, String> {
    command_result(state.preferences.lock().await.save(preferences))
}

#[tauri::command]
pub(crate) fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn test_notification(state: State<'_, DesktopState>) -> Result<(), String> {
    use chrono::Timelike;
    let client = state.runtime.lock().await.state().await;
    let server_id = client.selected_server_id.as_deref().unwrap_or("local");
    let now = chrono::Local::now();
    if state.preferences.lock().await.get().notifications_paused(
        &format!("{server_id}:notification-test"),
        now.timestamp(),
        now.hour() as u8,
    ) {
        return Err(
            "Notifications are paused by snooze or quiet hours. Resume them to test delivery."
                .into(),
        );
    }
    let request = serde_json::from_value(serde_json::json!({
        "id": "notification-test", "request_id": "notification-test", "channel_id": "notification-test",
        "title": "Nod notifications are on", "summary": "Your device accepted this test notification.",
        "status": "resolved", "options": [], "created_at": now, "updated_at": now
    })).map_err(|error| error.to_string())?;
    command_result(state.notifier.show(server_id, &request).await)
}

#[tauri::command]
pub(crate) async fn select_all_channels(
    state: State<'_, DesktopState>,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.select_all_channels().await)
}

#[tauri::command]
pub(crate) async fn submit_request_option(
    state: State<'_, DesktopState>,
    params: nod_client_core::SubmitRequestOptionParams,
) -> Result<nod_client_core::models::Request, String> {
    command_result(
        state
            .runtime
            .lock()
            .await
            .submit_request_option(params)
            .await,
    )
}

#[tauri::command]
pub(crate) async fn open_request(
    state: State<'_, DesktopState>,
    params: nod_client_core::OpenRequestParams,
) -> Result<ClientState, String> {
    command_result(state.runtime.lock().await.open_request(params).await)
}

#[tauri::command]
pub(crate) async fn query_history(
    state: State<'_, DesktopState>,
    params: nod_client_core::QueryHistoryParams,
) -> Result<nod_client_core::models::RequestsResponse, String> {
    command_result(state.runtime.lock().await.query_history(params).await)
}
