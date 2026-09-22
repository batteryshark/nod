import { useEffect, useMemo, useRef, useState } from "react";
import {
  clearChannel,
  enroll,
  forgetServer,
  getState,
  listDevices,
  openExternalUrl,
  refresh,
  renameDevice,
  revokeDevice,
  selectChannel,
  selectAllChannels,
  getDesktopPreferences,
  setDesktopPreferences,
  getAutostart,
  setAutostart,
  testNotification,
  openRequest,
  selectServer,
  setNotificationPreference,
  setSubscription,
  submitRequestOption as submitScopedOption,
} from "../commands";
import { listenForRuntimeMessages } from "../events";
import { replaceRequest, selectedChannel, selectedRequest } from "../domain";
import type {
  Channel,
  ClientState,
  EnrollParams,
  RequestOption,
  NodRequest,
  RuntimeMessage,
  ServerProfile,
  UserDevice,
} from "../types";
import {
  DEFAULT_DESKTOP_PREFERENCES,
  type DesktopPreferences,
} from "../dto/desktopPreferences";
import { EMPTY_CLIENT_STATE } from "./state";

export interface DesktopClientCommands {
  clearError: () => void;
  closeSettings: () => void;
  clearSelectedChannel: () => Promise<boolean>;
  enrollDevice: (params: EnrollParams) => Promise<boolean>;
  forgetSelectedServer: () => Promise<boolean>;
  openSettings: () => void;
  openUrl: (url: string) => Promise<void>;
  refreshState: () => Promise<void>;
  renameUserDevice: (deviceId: string, name: string) => Promise<boolean>;
  revokeUserDevice: (deviceId: string) => Promise<boolean>;
  selectAllChannels: () => Promise<void>;
  updatePreferences: (preferences: DesktopPreferences) => Promise<boolean>;
  updateAutostart: (enabled: boolean) => Promise<boolean>;
  testNotification: () => Promise<boolean>;
  selectChannel: (channel: Channel) => Promise<void>;
  selectRequest: (request: NodRequest) => Promise<void>;
  selectServer: (server: ServerProfile) => Promise<void>;
  submitRequestOption: (
    request: NodRequest,
    option: RequestOption,
    text?: string,
  ) => Promise<boolean>;
  toggleChannelSubscription: (channel: Channel) => Promise<void>;
  updateNotificationSound: (notificationSound: string) => Promise<void>;
}

export interface DesktopClient {
  activeChannel?: Channel;
  activeRequest?: NodRequest;
  commands: DesktopClientCommands;
  devices: UserDevice[];
  preferences: DesktopPreferences;
  autostart: boolean;
  error: string | null;
  isLoading: boolean;
  settingsOpen: boolean;
  state: ClientState;
}

export function useDesktopClient(): DesktopClient {
  const [state, setState] = useState<ClientState>(EMPTY_CLIENT_STATE);
  const [isLoading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [preferences, setPreferences] = useState(DEFAULT_DESKTOP_PREFERENCES);
  const [autostart, setAutostartValue] = useState(false);
  const [devices, setDevices] = useState<UserDevice[]>([]);
  const stateEvents = useRef(0);

  useEffect(() => {
    let cancelled = false;
    let stopListening: (() => void) | undefined;

    getDesktopPreferences()
      .then((value) => {
        if (!cancelled) setPreferences(value);
      })
      .catch((reason: unknown) => setError(String(reason)));
    getAutostart()
      .then((value) => {
        if (!cancelled) setAutostartValue(value);
      })
      .catch((reason: unknown) => setError(String(reason)));

    // Subscribe before taking the snapshot, and never let a delayed command
    // response overwrite a newer state event from the shared runtime.
    async function start(): Promise<void> {
      try {
        const unlisten = await listenForRuntimeMessages((message) => {
          if (cancelled) return;
          if (message.kind === "state") stateEvents.current += 1;
          applyRuntimeMessage(message, setState, setError);
        });
        if (cancelled) {
          unlisten();
          return;
        }
        stopListening = unlisten;
        const revision = stateEvents.current;
        const loaded = await getState();
        if (!cancelled && stateEvents.current === revision) setState(loaded);
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void start();

    return () => {
      cancelled = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    if (!settingsOpen) {
      return;
    }
    let cancelled = false;
    setDevices([]);
    listDevices()
      .then((value) => {
        if (!cancelled) setDevices(value);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [settingsOpen, state.selected_server_id]);

  const activeChannel = useMemo(() => selectedChannel(state), [state]);
  const activeRequest = useMemo(() => selectedRequest(state), [state]);

  async function runStateCommand(
    work: () => Promise<ClientState>,
  ): Promise<boolean> {
    try {
      setError(null);
      const revision = stateEvents.current;
      const updated = await work();
      if (stateEvents.current === revision) setState(updated);
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    }
  }

  async function enrollDevice(params: EnrollParams): Promise<boolean> {
    return runStateCommand(() => enroll(params));
  }

  async function refreshState(): Promise<void> {
    await runStateCommand(refresh);
  }

  async function selectServerProfile(server: ServerProfile): Promise<void> {
    await runStateCommand(() => selectServer({ server_id: server.id }));
  }

  async function selectNodChannel(channel: Channel): Promise<void> {
    await runStateCommand(() => selectChannel({ channel_id: channel.id }));
  }

  async function selectNodRequest(request: NodRequest): Promise<void> {
    await runStateCommand(() =>
      openRequest({
        server_id: state.selected_server_id ?? "",
        request_id: request.id,
      }),
    );
  }

  async function submitRequestOption(
    request: NodRequest,
    option: RequestOption,
    text?: string,
  ): Promise<boolean> {
    try {
      setError(null);
      const revision = stateEvents.current;
      const updated = await submitScopedOption({
        server_id: state.selected_server_id ?? "",
        request_id: request.id,
        option_id: option.id,
        text,
      });
      if (stateEvents.current === revision)
        setState((current) => ({
          ...current,
          requests:
            current.selected_server_id === state.selected_server_id
              ? replaceRequest(current.requests, updated)
              : current.requests,
        }));
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    }
  }

  async function openUrl(url: string): Promise<void> {
    try {
      setError(null);
      await openExternalUrl(url);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function updateNotificationSound(
    notificationSound: string,
  ): Promise<void> {
    await runStateCommand(() =>
      setNotificationPreference({ notification_sound: notificationSound }),
    );
  }

  async function toggleChannelSubscription(channel: Channel): Promise<void> {
    await runStateCommand(() =>
      setSubscription({
        channel_id: channel.id,
        subscribed: !channel.subscribed,
      }),
    );
  }

  async function renameUserDevice(
    deviceId: string,
    name: string,
  ): Promise<boolean> {
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      return false;
    }
    try {
      setError(null);
      const device = await renameDevice({
        device_id: deviceId,
        name: trimmedName,
      });
      setDevices((current) =>
        current.map((candidate) =>
          candidate.id === device.id ? device : candidate,
        ),
      );
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    }
  }

  async function revokeUserDevice(deviceId: string): Promise<boolean> {
    if (!(await runStateCommand(() => revokeDevice({ device_id: deviceId }))))
      return false;
    setDevices((current) => current.filter((device) => device.id !== deviceId));
    return true;
  }

  async function forgetSelectedServer(): Promise<boolean> {
    const serverId = state.selected_server_id;
    if (!serverId) return false;
    const success = await runStateCommand(() =>
      forgetServer({ server_id: serverId }),
    );
    if (success) setSettingsOpen(false);
    return success;
  }

  async function clearSelectedChannel(): Promise<boolean> {
    const channelId = state.selected_channel_id;
    if (!channelId) return false;
    return runStateCommand(() => clearChannel({ channel_id: channelId }));
  }

  return {
    activeChannel,
    activeRequest,
    commands: {
      clearError: () => setError(null),
      clearSelectedChannel,
      closeSettings: () => setSettingsOpen(false),
      enrollDevice,
      forgetSelectedServer,
      openSettings: () => setSettingsOpen(true),
      openUrl,
      refreshState,
      renameUserDevice,
      revokeUserDevice,
      selectAllChannels: async () => {
        await runStateCommand(selectAllChannels);
      },
      updatePreferences: async (value) => {
        try {
          setPreferences(await setDesktopPreferences(value));
          return true;
        } catch (reason) {
          setError(String(reason));
          return false;
        }
      },
      updateAutostart: async (enabled) => {
        try {
          await setAutostart(enabled);
          setAutostartValue(enabled);
          return true;
        } catch (reason) {
          setError(String(reason));
          return false;
        }
      },
      testNotification: async () => {
        try {
          await testNotification();
          return true;
        } catch (reason) {
          setError(String(reason));
          return false;
        }
      },
      selectChannel: selectNodChannel,
      selectRequest: selectNodRequest,
      selectServer: selectServerProfile,
      submitRequestOption,
      toggleChannelSubscription,
      updateNotificationSound,
    },
    devices,
    preferences,
    autostart,
    error,
    isLoading,
    settingsOpen,
    state,
  };
}

function applyRuntimeMessage(
  runtimeMessage: RuntimeMessage,
  setState: (state: ClientState) => void,
  setError: (message: string | null) => void,
): void {
  switch (runtimeMessage.kind) {
    case "state":
      setState(runtimeMessage.payload);
      setError(runtimeMessage.payload.last_error ?? null);
      break;
    case "transient_error":
      setError(runtimeMessage.payload.message);
      break;
    case "auth_revoked":
      setError("This device registration was revoked.");
      break;
    default:
      break;
  }
}
