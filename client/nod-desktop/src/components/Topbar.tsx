import { X } from "lucide-react";
import type { Channel, SyncPhase } from "../types";
interface TopbarProps {
  activeChannel?: Channel;
  error: string | null;
  phase: SyncPhase;
  lastSyncedAt?: string | null;
  onDismissError: () => void;
  onRetry: () => Promise<void>;
}
const phaseLabels: Record<SyncPhase, string> = {
  offline: "Offline · showing saved requests",
  connecting: "Connecting…",
  reconciling: "Updating inbox…",
  current: "Up to date",
  revoked: "Device access revoked",
};
export function Topbar({
  activeChannel,
  error,
  phase,
  lastSyncedAt,
  onDismissError,
  onRetry,
}: TopbarProps): JSX.Element {
  return (
    <header className="topbar">
      <div>
        <p>{activeChannel?.name ?? "All channels"}</p>
        <span role="status">{phaseLabels[phase]}</span>
        {lastSyncedAt ? (
          <small> · Synced {new Date(lastSyncedAt).toLocaleTimeString()}</small>
        ) : null}
      </div>
      {error ? (
        <div className="alert">
          <p role="alert">{error}</p>
          <button type="button" onClick={() => void onRetry()}>
            Retry
          </button>
          <button
            type="button"
            aria-label="Dismiss error"
            onClick={onDismissError}
          >
            <X size={14} />
          </button>
        </div>
      ) : null}
    </header>
  );
}
