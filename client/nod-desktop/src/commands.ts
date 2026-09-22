import { invoke } from "@tauri-apps/api/core";
import type {
  ClientState,
  EnrollParams,
  NodRequest,
  NotificationPreferenceParams,
  RenameDeviceParams,
  RevokeDeviceParams,
  SelectRequestParams,
  SelectServerParams,
  SetSubscriptionParams,
  ChannelParams,
  SubmitOptionParams,
  UserDevice,
} from "./types";

export function getState(): Promise<ClientState> {
  return invoke<ClientState>("state");
}

export function enroll(params: EnrollParams): Promise<ClientState> {
  return invoke<ClientState>("enroll", { params });
}

export function refresh(): Promise<ClientState> {
  return invoke<ClientState>("refresh");
}

export function selectServer(params: SelectServerParams): Promise<ClientState> {
  return invoke<ClientState>("select_server", { params });
}

export function forgetServer(params: SelectServerParams): Promise<ClientState> {
  return invoke<ClientState>("forget_server", { params });
}

export function selectChannel(params: ChannelParams): Promise<ClientState> {
  return invoke<ClientState>("select_channel", { params });
}

export function selectRequest(
  params: SelectRequestParams,
): Promise<ClientState> {
  return invoke<ClientState>("select_request", { params });
}

export function submitOption(params: SubmitOptionParams): Promise<NodRequest> {
  return invoke<NodRequest>("submit_option", { params });
}

export function clearChannel(params: ChannelParams): Promise<ClientState> {
  return invoke<ClientState>("clear_channel", { params });
}

export function setSubscription(
  params: SetSubscriptionParams,
): Promise<ClientState> {
  return invoke<ClientState>("set_subscription", { params });
}

export function setNotificationPreference(
  params: NotificationPreferenceParams,
): Promise<ClientState> {
  return invoke<ClientState>("set_notification_preference", { params });
}

export function listDevices(): Promise<UserDevice[]> {
  return invoke<UserDevice[]>("list_devices");
}

export function renameDevice(params: RenameDeviceParams): Promise<UserDevice> {
  return invoke<UserDevice>("rename_device", { params });
}

export function revokeDevice(params: RevokeDeviceParams): Promise<ClientState> {
  return invoke<ClientState>("revoke_device", { params });
}

export function openExternalUrl(url: string): Promise<void> {
  return invoke<void>("open_external_url", { url });
}

export function selectAllChannels(): Promise<ClientState> {
  return invoke<ClientState>("select_all_channels");
}

export function getDesktopPreferences(): Promise<
  import("./dto/desktopPreferences").DesktopPreferences
> {
  return invoke("desktop_preferences");
}

export function setDesktopPreferences(
  preferences: import("./dto/desktopPreferences").DesktopPreferences,
): Promise<import("./dto/desktopPreferences").DesktopPreferences> {
  return invoke("set_desktop_preferences", { preferences });
}

export function getAutostart(): Promise<boolean> {
  return invoke("autostart_enabled");
}
export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}
export function testNotification(): Promise<void> {
  return invoke("test_notification");
}

export function submitRequestOption(
  params: SubmitOptionParams & { server_id: string },
): Promise<NodRequest> {
  return invoke("submit_request_option", { params });
}

export function openRequest(params: {
  server_id: string;
  request_id: string;
}): Promise<ClientState> {
  return invoke("open_request", { params });
}
export function queryHistory(params: {
  server_id: string;
  channel_id?: string;
  search?: string;
  before?: string;
  limit?: number;
}): Promise<{ requests: NodRequest[]; next_cursor?: string | null }> {
  return invoke("query_history", { params });
}

export function requestImage(url: string): Promise<string> {
  return invoke("request_image", { url });
}
