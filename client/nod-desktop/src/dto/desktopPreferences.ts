export interface DesktopPreferences {
  hide_notification_content: boolean;
  load_remote_images: boolean;
  muted_channels: string[];
  snoozed_until: number | null;
  quiet_start_hour: number | null;
  quiet_end_hour: number | null;
}
export const DEFAULT_DESKTOP_PREFERENCES: DesktopPreferences = {
  hide_notification_content: false,
  load_remote_images: false,
  muted_channels: [],
  snoozed_until: null,
  quiet_start_hour: null,
  quiet_end_hour: null,
};
