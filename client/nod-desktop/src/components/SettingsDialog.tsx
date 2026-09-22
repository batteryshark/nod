import { Modal } from "./Modal";
import { NotificationSettings } from "./settings/NotificationSettings";
import type { DesktopPreferences } from "../dto/desktopPreferences";
import { X } from "lucide-react";
import { NOTIFICATION_SOUND_OPTIONS } from "../app/state";
import type { Channel, ClientState, UserDevice } from "../types";
import { ChannelSubscriptions } from "./settings/ChannelSubscriptions";
import { DestructiveSettingsControls } from "./settings/DestructiveSettingsControls";
import { DeviceList } from "./settings/DeviceList";

export interface SettingsDialogCommands {
  clearSelectedChannel: () => Promise<boolean>;
  closeSettings: () => void;
  forgetSelectedServer: () => Promise<boolean>;
  renameUserDevice: (deviceId: string, name: string) => Promise<boolean>;
  revokeUserDevice: (deviceId: string) => Promise<boolean>;
  toggleChannelSubscription: (channel: Channel) => Promise<void>;
  updatePreferences: (preferences: DesktopPreferences) => Promise<boolean>;
  updateAutostart: (enabled: boolean) => Promise<boolean>;
  testNotification: () => Promise<boolean>;
  updateNotificationSound: (notificationSound: string) => Promise<void>;
}

interface SettingsDialogProps {
  commands: SettingsDialogCommands;
  devices: UserDevice[];
  state: ClientState;
  preferences: DesktopPreferences;
  autostart: boolean;
  error: string | null;
}

export function SettingsDialog({
  commands,
  devices,
  state,
  preferences,
  autostart,
  error,
}: SettingsDialogProps): JSX.Element {
  return (
    <Modal title="Settings" onClose={commands.closeSettings}>
      <header>
        <h2>Settings</h2>
        <button
          type="button"
          aria-label="Close settings"
          onClick={commands.closeSettings}
        >
          <X size={16} />
        </button>
      </header>
      <label>
        Notification Sound
        <select
          value={state.notification_sound}
          onChange={(event) =>
            void commands.updateNotificationSound(event.currentTarget.value)
          }
        >
          {NOTIFICATION_SOUND_OPTIONS.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <NotificationSettings
        preferences={preferences}
        autostart={autostart}
        state={state}
        onSave={commands.updatePreferences}
        onAutostart={commands.updateAutostart}
        onTest={commands.testNotification}
      />
      <ChannelSubscriptions
        channels={state.channels}
        onToggleChannel={commands.toggleChannelSubscription}
      />
      <DeviceList
        devices={devices}
        onRenameDevice={commands.renameUserDevice}
        onRevokeDevice={commands.revokeUserDevice}
      />
      <DestructiveSettingsControls
        serverName={
          state.servers.find((server) => server.id === state.selected_server_id)
            ?.name ?? "this server"
        }
        channelName={
          state.channels.find(
            (channel) => channel.id === state.selected_channel_id,
          )?.name ?? "this channel"
        }
        canClearChannel={Boolean(state.selected_channel_id)}
        onClearSelectedChannel={commands.clearSelectedChannel}
        onForgetSelectedServer={commands.forgetSelectedServer}
      />
      {error ? (
        <p role="alert" className="formError">
          {error}
        </p>
      ) : null}
    </Modal>
  );
}
