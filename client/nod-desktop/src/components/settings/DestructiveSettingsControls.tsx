import { Trash2 } from "lucide-react";
import { useState } from "react";
interface Props {
  canClearChannel: boolean;
  serverName: string;
  channelName: string;
  onClearSelectedChannel: () => Promise<boolean>;
  onForgetSelectedServer: () => Promise<boolean>;
}
export function DestructiveSettingsControls({
  canClearChannel,
  serverName,
  channelName,
  onClearSelectedChannel,
  onForgetSelectedServer,
}: Props): JSX.Element {
  const [confirm, setConfirm] = useState<"forget" | "clear" | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <footer>
      <button
        type="button"
        className="danger"
        disabled={busy}
        onClick={() => setConfirm("forget")}
      >
        <Trash2 size={16} />
        Forget server
      </button>
      <button
        type="button"
        disabled={busy || !canClearChannel}
        onClick={() => setConfirm("clear")}
      >
        <Trash2 size={16} />
        Clear handled requests
      </button>
      {confirm ? (
        <div role="group" aria-label="Confirm action">
          <p>
            {confirm === "forget"
              ? `Forget ${serverName} on this device? You will need a new enrollment code to reconnect. Other devices keep their access.`
              : `Hide handled requests in ${channelName} for your account? Pending requests remain available. You can find handled requests again using server history.`}
          </p>
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                const success =
                  confirm === "forget"
                    ? await onForgetSelectedServer()
                    : await onClearSelectedChannel();
                if (success) setConfirm(null);
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? "Working…" : "Confirm"}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => setConfirm(null)}
          >
            Cancel
          </button>
        </div>
      ) : null}
    </footer>
  );
}
