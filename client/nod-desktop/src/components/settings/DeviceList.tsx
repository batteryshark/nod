import { Check, ChevronDown, Trash2 } from "lucide-react";
import { useState } from "react";
import type { UserDevice } from "../../types";

interface DeviceListProps {
  devices: UserDevice[];
  onRenameDevice: (deviceId: string, name: string) => Promise<boolean>;
  onRevokeDevice: (deviceId: string) => Promise<boolean>;
}

export function DeviceList({
  devices,
  onRenameDevice,
  onRevokeDevice,
}: DeviceListProps): JSX.Element {
  const [renamingDeviceId, setRenamingDeviceId] = useState<string | null>(null);
  const [revokeId, setRevokeId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [renameText, setRenameText] = useState("");

  async function renameSelectedDevice(): Promise<void> {
    if (renamingDeviceId === null) {
      return;
    }
    if (await onRenameDevice(renamingDeviceId, renameText)) {
      setRenamingDeviceId(null);
      setRenameText("");
    }
  }

  return (
    <section className="settingsSection">
      <h3>Devices</h3>
      {devices.map((device) => (
        <div className="deviceRow" key={device.id}>
          {renamingDeviceId === device.id ? (
            <input
              aria-label="Device name"
              value={renameText}
              onChange={(event) => setRenameText(event.currentTarget.value)}
            />
          ) : (
            <span>{device.name}</span>
          )}
          <small>
            {device.platform}
            {device.is_current ? " current" : ""}
          </small>
          {renamingDeviceId === device.id ? (
            <button
              type="button"
              aria-label="Save device name"
              onClick={() => void renameSelectedDevice()}
              disabled={renameText.trim().length === 0}
            >
              <Check size={14} />
            </button>
          ) : (
            <button
              type="button"
              aria-label={`Rename ${device.name}`}
              onClick={() => {
                setRenamingDeviceId(device.id);
                setRenameText(device.name);
              }}
            >
              <ChevronDown size={14} />
            </button>
          )}
          <button
            type="button"
            className="dangerIcon"
            aria-label={`Revoke ${device.name}`}
            onClick={() => setRevokeId(device.id)}
          >
            <Trash2 size={14} />
          </button>
          {revokeId === device.id ? (
            <div role="group" aria-label="Confirm revocation">
              <p>
                Revoke {device.name}?{" "}
                {device.is_current
                  ? "This device will disconnect immediately."
                  : "It will lose access immediately."}{" "}
                A new enrollment code is required to reconnect.
              </p>
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    if (await onRevokeDevice(device.id)) setRevokeId(null);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Revoke device
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => setRevokeId(null)}
              >
                Cancel
              </button>
            </div>
          ) : null}
        </div>
      ))}
    </section>
  );
}
