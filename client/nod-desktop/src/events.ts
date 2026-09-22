import { listen } from "@tauri-apps/api/event";
import type { RuntimeMessage } from "./types";

const RUNTIME_EVENT_NAME = "nod://request";

export function listenForRuntimeMessages(
  handler: (message: RuntimeMessage) => void,
): Promise<() => void> {
  return listen<RuntimeMessage>(RUNTIME_EVENT_NAME, (message) =>
    handler(message.payload),
  );
}
