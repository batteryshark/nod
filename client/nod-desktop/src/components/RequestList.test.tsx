// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(cleanup);
import { RequestList } from "./RequestList";
import type { NodRequest, RequestStatus } from "../types";

function request(id: string, status: RequestStatus): NodRequest {
  return {
    id,
    request_id: id,
    channel_id: "default",
    recipients: ["owner"],
    decision_resolution: "shared",
    title: `Request ${id}`,
    summary: "summary",
    body_markdown: "",
    fields: [],
    links: [],
    image_url: null,
    notification: { redact: false, title: null, body: null },
    dedupe_key: null,
    expires_at: null,
    status,
    created_at: "2026-06-10T12:00:00.000Z",
    updated_at: "2026-06-10T12:00:00.000Z",
    resolved_at: null,
    decision: null,
    decisions: [],
    callback_url: null,
    options: [],
    request_digest: "digest",
  };
}

describe("RequestList sections", () => {
  it("shows pending expanded and handled collapsed by default", () => {
    render(
      <RequestList
        serverId="server-1"
        requests={[request("a", "pending"), request("b", "resolved")]}
        onSelect={vi.fn()}
        selectedRequestId={null}
      />,
    );

    expect(screen.getByText("Request a")).toBeInTheDocument();
    expect(screen.queryByText("Request b")).toBeNull();
    expect(screen.getByRole("button", { name: /Pending\s*1/ })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByRole("button", { name: /Handled\s*1/ })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("toggles a section from its header", () => {
    render(
      <RequestList
        serverId="server-1"
        requests={[request("a", "pending"), request("b", "resolved")]}
        onSelect={vi.fn()}
        selectedRequestId={null}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Handled\s*1/ }));
    expect(screen.getByText("Request b")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Pending\s*1/ }));
    expect(screen.queryByText("Request a")).toBeNull();
  });

  it("expands handled when nothing is pending", () => {
    render(
      <RequestList
        serverId="server-1"
        requests={[request("b", "resolved"), request("c", "expired")]}
        onSelect={vi.fn()}
        selectedRequestId={null}
      />,
    );

    expect(screen.getByText("Request b")).toBeInTheDocument();
    expect(screen.getByText("Request c")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Pending/ })).toBeNull();
  });

  it("hides both section headers when there are no requests", () => {
    render(
      <RequestList
        serverId="server-1"
        requests={[]}
        onSelect={vi.fn()}
        selectedRequestId={null}
      />,
    );

    expect(screen.getByText("No Requests")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Pending|Handled/ }),
    ).toBeNull();
  });
});
