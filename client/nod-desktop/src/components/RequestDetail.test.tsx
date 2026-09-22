// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(cleanup);
import { RequestDetail } from "./RequestDetail";
import type { NodRequest, RequestOption } from "../types";

const approve: RequestOption = {
  id: "approve",
  label: "Approve",
  kind: "approve",
  style: "primary",
  requires_text: false,
  text_placeholder: null,
  destructive: false,
  foreground: false,
};

const approveWithNotes: RequestOption = {
  ...approve,
  id: "approve_notes",
  label: "Approve with notes",
  kind: "approve_with_text",
};

const reject: RequestOption = {
  ...approve,
  id: "reject",
  label: "Reject",
  kind: "reject",
  destructive: true,
};

function pendingRequest(options: RequestOption[]): NodRequest {
  return {
    id: "req-1",
    request_id: "req-1",
    channel_id: "default",
    recipients: ["owner"],
    decision_resolution: "shared",
    title: "Deploy?",
    summary: "v1 is ready",
    body_markdown: "",
    fields: [],
    links: [],
    image_url: null,
    notification: { redact: false, title: null, body: null },
    dedupe_key: null,
    expires_at: null,
    status: "pending",
    created_at: "2026-06-10T12:00:00.000Z",
    updated_at: "2026-06-10T12:00:00.000Z",
    resolved_at: null,
    decision: null,
    decisions: [],
    callback_url: null,
    options,
    request_digest: "digest",
  };
}

describe("RequestDetail notes", () => {
  it("sends the shared notes with whichever option is clicked", async () => {
    const onOption = vi.fn().mockResolvedValue(true);
    render(
      <RequestDetail
        request={pendingRequest([approve, approveWithNotes, reject])}
        onOption={onOption}
        onOpenUrl={vi.fn()}
      />,
    );

    // Typing must not crash (regression: reading event.currentTarget inside
    // the state updater blanked the screen on the first keystroke).
    const notes = screen.getByPlaceholderText(
      "Sent with whichever decision you pick",
    );
    fireEvent.change(notes, { target: { value: "ship it carefully" } });

    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(onOption).toHaveBeenCalledWith(
      expect.objectContaining({ id: "req-1" }),
      expect.objectContaining({ id: "reject" }),
      "ship it carefully",
    );
  });

  it("preserves distinct option labels and submits their exact IDs", async () => {
    const onOption = vi.fn().mockResolvedValue(true);
    render(
      <RequestDetail
        request={pendingRequest([approve, approveWithNotes, reject])}
        onOption={onOption}
        onOpenUrl={vi.fn()}
      />,
    );
    const notesAction = screen.getByRole("button", {
      name: "Approve with notes",
    });
    expect(notesAction).toBeEnabled();
    fireEvent.change(
      screen.getByPlaceholderText("Sent with whichever decision you pick"),
      { target: { value: "ship it" } },
    );
    fireEvent.click(notesAction);
    await waitFor(() =>
      expect(onOption).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ id: "approve_notes" }),
        "ship it",
      ),
    );
  });

  it("retains failed drafts across navigation without crossing server identities", async () => {
    const onOption = vi.fn().mockResolvedValue(false);
    const request = pendingRequest([approveWithNotes]);
    const props = { onOption, onOpenUrl: vi.fn() };
    const view = render(
      <RequestDetail request={request} serverId="first" {...props} />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "keep this" },
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: "Approve with notes",
      }),
    );
    await screen.findByText(/Decision was not confirmed/);
    view.rerender(
      <RequestDetail request={request} serverId="second" {...props} />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("");
    view.rerender(
      <RequestDetail request={request} serverId="first" {...props} />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("keep this");
  });

  it("completes the originating draft after navigating to another server", async () => {
    let resolve!: (value: boolean) => void;
    const onOption = vi.fn(
      () =>
        new Promise<boolean>((done) => {
          resolve = done;
        }),
    );
    const request = pendingRequest([approveWithNotes]);
    const props = { request, onOption, onOpenUrl: vi.fn() };
    const view = render(<RequestDetail serverId="first" {...props} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "first draft" },
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: "Approve with notes",
      }),
    );
    view.rerender(<RequestDetail serverId="second" {...props} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "second draft" },
    });
    await act(async () => resolve(true));
    expect(screen.getByRole("textbox")).toHaveValue("second draft");
    expect(
      screen.queryByText("Approve with notes recorded."),
    ).not.toBeInTheDocument();
    view.rerender(<RequestDetail serverId="first" {...props} />);
    expect(screen.getByRole("textbox")).toHaveValue("");
    expect(
      screen.getByText("Approve with notes recorded."),
    ).toBeInTheDocument();
  });

  it("prevents duplicate submissions while a decision is in flight", async () => {
    let resolve!: (value: boolean) => void;
    const onOption = vi.fn(
      () =>
        new Promise<boolean>((done) => {
          resolve = done;
        }),
    );
    render(
      <RequestDetail
        request={pendingRequest([approve])}
        onOption={onOption}
        onOpenUrl={vi.fn()}
      />,
    );
    const button = screen.getByRole("button", { name: "Approve" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(onOption).toHaveBeenCalledTimes(1);
    expect(button).toBeDisabled();
    await act(async () => resolve(true));
    expect(button).toBeEnabled();
  });

  it("renders Markdown without scripts, unsafe links or tracking images", () => {
    const request = {
      ...pendingRequest([approve]),
      body_markdown:
        "**Review** <script>alert(1)</script> [bad](javascript:alert) ![tracking](https://tracker.test/image.png)",
      image_url: "https://tracker.test/a.png",
    };
    const { container } = render(
      <RequestDetail
        request={request}
        onOption={vi.fn()}
        onOpenUrl={vi.fn()}
      />,
    );
    expect(container.querySelector("strong")).toHaveTextContent("Review");
    expect(
      container.querySelector("script, img, a[href^='javascript:']"),
    ).toBeNull();
  });

  it("accepts empty optional notes for a text-capable option", async () => {
    const onOption = vi.fn().mockResolvedValue(true);
    render(
      <RequestDetail
        request={pendingRequest([approveWithNotes, reject])}
        onOption={onOption}
        onOpenUrl={vi.fn()}
      />,
    );

    const withNotes = screen.getByRole("button", {
      name: "Approve with notes",
    });
    expect(withNotes).toBeEnabled();
    fireEvent.click(withNotes);
    expect(onOption).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({ id: "approve_notes" }),
      undefined,
    );

    await screen.findByText("Approve with notes recorded.");
    fireEvent.change(
      screen.getByPlaceholderText("Sent with whichever decision you pick"),
      { target: { value: "lgtm" } },
    );
    await waitFor(() => expect(withNotes).toBeEnabled());
  });

  it("hides the notes field when no option accepts text", () => {
    render(
      <RequestDetail
        request={pendingRequest([approve, reject])}
        onOption={vi.fn()}
        onOpenUrl={vi.fn()}
      />,
    );

    expect(
      screen.queryByPlaceholderText("Sent with whichever decision you pick"),
    ).toBeNull();
  });
});
