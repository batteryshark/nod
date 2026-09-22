import { Check } from "lucide-react";
import { FormEvent, useState } from "react";
import nodIcon from "../assets/nod-icon.png";
import { NOTIFICATION_SOUND_OPTIONS } from "../app/state";
import { canSubmitEnrollment, parseEnrollmentLink } from "../domain";
import type { EnrollParams } from "../types";

interface EnrollmentViewProps {
  error: string | null;
  onCancel?: () => void;
  onEnroll: (params: EnrollParams) => Promise<boolean>;
}

export function EnrollmentView({
  error,
  onEnroll,
  onCancel,
}: EnrollmentViewProps): JSX.Element {
  const [busy, setBusy] = useState(false);
  const [linkError, setLinkError] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [deviceName, setDeviceName] = useState(navigator.platform || "Desktop");
  const [code, setCode] = useState("");
  const [notificationSound, setNotificationSound] = useState("default");

  const enrollmentDraft = {
    base_url: baseUrl,
    device_name: deviceName,
    code,
  };

  async function submit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      if (
        await onEnroll({
          ...enrollmentDraft,
          notification_sound: notificationSound,
        })
      )
        onCancel?.();
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="enrollment">
      <form
        className="enrollmentPanel"
        onSubmit={(event) => void submit(event)}
      >
        <div className="enrollmentHeader">
          <img src={nodIcon} alt="" />
          <h1>Nod</h1>
        </div>
        <p>
          Ask your server administrator for a one-use enrollment code or link.
          Codes normally expire after ten minutes. Registration connects this
          device to your account.
        </p>
        <label>
          Enrollment link (optional)
          <input
            placeholder="nod://enroll?server=…&code=…"
            onChange={(event) => {
              const value = event.currentTarget.value.trim();
              const parsed = parseEnrollmentLink(value);
              if (parsed) {
                setBaseUrl(parsed.base_url);
                setCode(parsed.code);
                setLinkError("");
              } else
                setLinkError(
                  value
                    ? "Use a Nod enrollment link with a valid server and eight-character code."
                    : "",
                );
            }}
          />
        </label>
        {linkError ? <p role="alert">{linkError}</p> : null}
        <label>
          Server
          <input
            type="url"
            autoFocus
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.currentTarget.value)}
            placeholder="https://nod.example.com"
          />
        </label>
        <label>
          Device
          <input
            value={deviceName}
            onChange={(event) => setDeviceName(event.currentTarget.value)}
          />
        </label>
        <label>
          Code
          <input
            value={code}
            onChange={(event) =>
              setCode(event.currentTarget.value.toUpperCase())
            }
            maxLength={8}
            autoComplete="one-time-code"
          />
        </label>
        <label>
          Sound
          <select
            value={notificationSound}
            onChange={(event) =>
              setNotificationSound(event.currentTarget.value)
            }
          >
            {NOTIFICATION_SOUND_OPTIONS.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        {error ? (
          <p role="alert" className="formError">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={busy || !canSubmitEnrollment(enrollmentDraft)}
        >
          <Check size={16} />
          {busy ? "Registering…" : "Register device"}
        </button>
        {onCancel ? (
          <button type="button" disabled={busy} onClick={onCancel}>
            Cancel
          </button>
        ) : null}
      </form>
    </main>
  );
}
