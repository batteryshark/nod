import type { ClientState } from "../types";

export const EMPTY_CLIENT_STATE: ClientState = {
  servers: [],
  selected_server_id: null,
  current_user: null,
  devices: [],
  channels: [],
  pending_counts_by_channel: {},
  requests: [],
  selected_channel_id: null,
  selected_request_id: null,
  notification_sound: "default",
  notification_delivery_mode: "websocket",
  is_registered: false,
  is_sync_connected: false,
  sync_phase: "offline",
  last_synced_at: null,
  last_error: null,
};

export const NOTIFICATION_SOUND_OPTIONS = [
  { id: "default", label: "Default" },
  { id: "silent", label: "Silent" },
] as const;
