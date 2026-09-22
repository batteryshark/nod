import { useState } from "react";
import type { DesktopPreferences } from "../../dto/desktopPreferences";
import type { ClientState } from "../../types";
interface Props {
  preferences: DesktopPreferences;
  autostart: boolean;
  state: ClientState;
  onSave: (value: DesktopPreferences) => Promise<boolean>;
  onAutostart: (enabled: boolean) => Promise<boolean>;
  onTest: () => Promise<boolean>;
}
export function NotificationSettings({
  preferences,
  autostart,
  state,
  onSave,
  onAutostart,
  onTest,
}: Props): JSX.Element {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");
  async function save(next: Partial<DesktopPreferences>): Promise<void> {
    setBusy(true);
    try {
      await onSave({ ...preferences, ...next });
    } finally {
      setBusy(false);
    }
  }
  const snoozed = (preferences.snoozed_until ?? 0) > Date.now() / 1000;
  return (
    <section className="settingsSection">
      <h3>This device</h3>
      <fieldset disabled={busy}>
        <label className="checkRow">
          <input
            type="checkbox"
            checked={autostart}
            onChange={(event) => void onAutostart(event.currentTarget.checked)}
          />
          Start Nod when I sign in
        </label>
        <label className="checkRow">
          <input
            type="checkbox"
            checked={preferences.hide_notification_content}
            onChange={(event) =>
              void save({
                hide_notification_content: event.currentTarget.checked,
              })
            }
          />
          Hide request content in notifications
        </label>
        <label className="checkRow">
          <input
            type="checkbox"
            checked={preferences.load_remote_images}
            onChange={(event) =>
              void save({ load_remote_images: event.currentTarget.checked })
            }
          />
          Load remote request images
        </label>
        <small>
          Remote images contact the sender’s image host. Inline Markdown images
          remain hidden.
        </small>
        <button
          type="button"
          onClick={() =>
            void save({
              snoozed_until: snoozed
                ? null
                : Math.floor(Date.now() / 1000) + 3600,
            })
          }
        >
          {snoozed ? "Resume notifications" : "Snooze for one hour"}
        </button>
        {snoozed ? (
          <small>
            Paused until{" "}
            {new Date(preferences.snoozed_until! * 1000).toLocaleTimeString()}
          </small>
        ) : null}
        <label className="checkRow">
          <input
            type="checkbox"
            checked={preferences.quiet_start_hour !== null}
            onChange={(event) =>
              void save({
                quiet_start_hour: event.currentTarget.checked ? 22 : null,
                quiet_end_hour: event.currentTarget.checked ? 8 : null,
              })
            }
          />
          Quiet hours (this device’s local time)
        </label>
        {preferences.quiet_start_hour !== null ? (
          <div className="quietHours">
            <label>
              From
              <select
                value={preferences.quiet_start_hour}
                onChange={(event) =>
                  void save({
                    quiet_start_hour: Number(event.currentTarget.value),
                  })
                }
              >
                {Array.from({ length: 24 }, (_, hour) => (
                  <option key={hour} value={hour}>
                    {String(hour).padStart(2, "0")}:00
                  </option>
                ))}
              </select>
            </label>
            <label>
              Until
              <select
                value={preferences.quiet_end_hour!}
                onChange={(event) =>
                  void save({
                    quiet_end_hour: Number(event.currentTarget.value),
                  })
                }
              >
                {Array.from({ length: 24 }, (_, hour) => (
                  <option key={hour} value={hour}>
                    {String(hour).padStart(2, "0")}:00
                  </option>
                ))}
              </select>
            </label>
          </div>
        ) : null}
        <h4>Mute notifications by channel</h4>
        <small>Muted requests still appear in your inbox.</small>
        {state.channels.map((channel) => {
          const key = `${state.selected_server_id}:${channel.id}`;
          return (
            <label className="checkRow" key={key}>
              <input
                type="checkbox"
                checked={preferences.muted_channels.includes(key)}
                onChange={(event) =>
                  void save({
                    muted_channels: event.currentTarget.checked
                      ? [...preferences.muted_channels, key]
                      : preferences.muted_channels.filter(
                          (value) => value !== key,
                        ),
                  })
                }
              />
              {channel.emoji} {channel.name}
            </label>
          );
        })}
        <button
          type="button"
          onClick={async () => {
            setBusy(true);
            try {
              setResult(
                (await onTest())
                  ? "Test sent to your system. If it does not appear, check Nod in system notification settings and Focus/Do Not Disturb."
                  : "Test failed. Check the error below.",
              );
            } finally {
              setBusy(false);
            }
          }}
        >
          Send test notification
        </button>
        <p role="status">{result}</p>
      </fieldset>
    </section>
  );
}
