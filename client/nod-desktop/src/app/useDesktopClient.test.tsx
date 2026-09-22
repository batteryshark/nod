import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../nod-client-core/tests/fixtures/runtime-messages.json";
import * as commands from "../commands";
import { listenForRuntimeMessages } from "../events";
import { App } from "../App";
import type { ClientState, RuntimeMessage } from "../types";
import { DEFAULT_DESKTOP_PREFERENCES } from "../dto/desktopPreferences";
import { useDesktopClient } from "./useDesktopClient";
vi.mock("../commands");
vi.mock("../events");
// Rust's runtime_wire test serializes this same fixture. Rendering it exercises
// the frontend projection's nullable values, enums and nested request fields.
const initial = fixture.messages[0].payload as ClientState;
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(commands.getState).mockResolvedValue(initial);
  vi.mocked(commands.getDesktopPreferences).mockResolvedValue(
    DEFAULT_DESKTOP_PREFERENCES,
  );
  vi.mocked(commands.getAutostart).mockResolvedValue(false);
  vi.mocked(listenForRuntimeMessages).mockResolvedValue(vi.fn());
});
afterEach(cleanup);
describe("runtime wire integration", () => {
  it("renders the frozen Rust state with exact option labels and all-channel scope", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Fixture request" });
    expect(
      screen.getByRole("button", { name: "Approve change" }),
    ).toBeEnabled();
    expect(screen.getByText("Up to date")).toBeInTheDocument();
    expect(screen.getByText("this change").tagName).toBe("STRONG");
    expect(fixture.messages[1].payload).toMatchObject({
      server_id: initial.selected_server_id,
      request: {
        id: initial.selected_request_id,
        signing: { version: "nod-request-v2" },
      },
    });
  });
  it("keeps a newer state event when the initial snapshot returns late", async () => {
    let resolve!: (state: ClientState) => void;
    vi.mocked(commands.getState).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const { result } = renderHook(useDesktopClient);
    await waitFor(() => expect(commands.getState).toHaveBeenCalled());
    const listener = vi.mocked(listenForRuntimeMessages).mock.calls[0][0];
    const newer: RuntimeMessage = {
      kind: "state",
      payload: {
        ...initial,
        requests: [{ ...initial.requests[0], status: "resolved" }],
      },
    };
    await act(async () => {
      listener(newer);
      resolve(initial);
    });
    expect(result.current.state.requests[0].status).toBe("resolved");
  });
  it("keeps a newer runtime event when a refresh result returns late", async () => {
    let resolve!: (state: ClientState) => void;
    vi.mocked(commands.refresh).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const { result } = renderHook(useDesktopClient);
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    const listener = vi.mocked(listenForRuntimeMessages).mock.calls[0][0];
    let refresh!: Promise<void>;
    act(() => {
      refresh = result.current.commands.refreshState();
    });
    await act(async () => {
      listener({
        kind: "state",
        payload: {
          ...initial,
          sync_phase: "revoked",
          is_sync_connected: false,
        },
      });
      resolve(initial);
      await refresh;
    });
    expect(result.current.state.sync_phase).toBe("revoked");
  });
});
