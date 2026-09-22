import { describe, expect, it } from "vitest";
import {
  decisionActions,
  parseEnrollmentLink,
  safeWebUrl,
  replaceRequest,
  submittableOptions,
  optionRequiresText,
  canSubmitEnrollment,
  requestPreview,
  orderedRequests,
  selectedChannel,
  selectedRequest,
  channelColor,
  totalPendingCount,
} from "./domain";
import type { Channel, ClientState, RequestOption, NodRequest } from "./types";

const baseRequest: NodRequest = {
  id: "base",
  request_id: "base",
  channel_id: "default",
  recipients: [],
  decision_resolution: "shared",
  title: "Base",
  summary: "",
  body_markdown: "",
  fields: [],
  links: [],
  image_url: null,
  notification: {
    redact: false,
    title: null,
    body: null,
  },
  dedupe_key: null,
  expires_at: null,
  status: "pending",
  created_at: "2026-05-31T12:00:00.000Z",
  updated_at: "2026-05-31T12:00:00.000Z",
  resolved_at: null,
  decision: null,
  decisions: [],
  callback_url: null,
  options: [],
  request_digest: "digest",
};

const baseState: ClientState = {
  servers: [],
  selected_server_id: null,
  current_user: null,
  devices: [],
  channels: [],
  pending_counts_by_channel: {},
  requests: [],
  selected_channel_id: null,
  selected_request_id: null,
  notification_sound: "default",
  notification_delivery_mode: "websocket",
  is_registered: false,
  is_sync_connected: false,
  sync_phase: "offline",
  last_error: null,
};

const baseChannel: Channel = {
  id: "default",
  name: "Default",
  emoji: "🔔",
  subscribed: true,
  created_at: "2026-05-31T12:00:00.000Z",
};

const baseOption: RequestOption = {
  id: "approve",
  label: "Approve",
  kind: "approve",
  style: "default",
  requires_text: false,
  text_placeholder: null,
  destructive: false,
  foreground: false,
};

describe("domain helpers", () => {
  it("totals pending counts across channels", () => {
    expect(
      totalPendingCount({
        ...baseState,
        pending_counts_by_channel: { a: 2, b: 3 },
      }),
    ).toBe(5);
  });

  it("generates stable display colors from channel identity", () => {
    expect(channelColor(baseChannel)).toBe(channelColor({ ...baseChannel }));
  });

  it("orders pending requests before handled requests", () => {
    const requests = orderedRequests([
      {
        ...baseRequest,
        id: "resolved",
        request_id: "resolved",
        status: "resolved",
      },
      {
        ...baseRequest,
        id: "pending",
        request_id: "pending",
        status: "pending",
      },
    ]);

    expect(requests.map((request) => request.id)).toEqual([
      "pending",
      "resolved",
    ]);
  });

  it("orders newer requests before older requests with the same status", () => {
    const requests = orderedRequests([
      {
        ...baseRequest,
        id: "older",
        request_id: "older",
        created_at: "2026-05-31T11:00:00.000Z",
      },
      {
        ...baseRequest,
        id: "newer",
        request_id: "newer",
        created_at: "2026-05-31T13:00:00.000Z",
      },
    ]);

    expect(requests.map((request) => request.id)).toEqual(["newer", "older"]);
  });

  it("selects the explicit request when it exists", () => {
    const selected = selectedRequest({
      ...baseState,
      selected_request_id: "target",
      requests: [
        { ...baseRequest, id: "fallback", request_id: "fallback" },
        { ...baseRequest, id: "target", request_id: "target" },
      ],
    });

    expect(selected?.id).toBe("target");
  });

  it("falls back to the first ordered request when the selected request is absent", () => {
    const selected = selectedRequest({
      ...baseState,
      selected_request_id: "missing",
      requests: [
        {
          ...baseRequest,
          id: "resolved",
          request_id: "resolved",
          status: "resolved",
        },
        {
          ...baseRequest,
          id: "pending",
          request_id: "pending",
          status: "pending",
        },
      ],
    });

    expect(selected?.id).toBe("pending");
  });

  it("selects the explicit channel when it exists", () => {
    const selected = selectedChannel({
      ...baseState,
      selected_channel_id: "target",
      channels: [
        { ...baseChannel, id: "fallback" },
        { ...baseChannel, id: "target" },
      ],
    });

    expect(selected?.id).toBe("target");
  });

  it("keeps all-channel scope when the selected channel is absent", () => {
    const selected = selectedChannel({
      ...baseState,
      selected_channel_id: "missing",
      channels: [
        { ...baseChannel, id: "fallback" },
        { ...baseChannel, id: "target" },
      ],
    });

    expect(selected).toBeUndefined();
  });

  it("creates a default dismiss option for requests without options", () => {
    const options = submittableOptions({ ...baseRequest, options: [] });

    expect(options[0]).toMatchObject({
      id: "dismiss",
      label: "Dismiss",
      kind: "dismiss",
    });
  });

  it("uses request options when the request defines them", () => {
    const options = submittableOptions({
      ...baseRequest,
      options: [baseOption],
    });

    expect(options).toEqual([baseOption]);
  });

  it("treats explicit text options as requiring text", () => {
    expect(optionRequiresText({ ...baseOption, requires_text: true })).toBe(
      true,
    );
  });

  it("treats text option kinds as requiring text", () => {
    expect(
      optionRequiresText({ ...baseOption, kind: "approve_with_text" }),
    ).toBe(true);
  });

  it("uses the request summary as the preview text", () => {
    expect(
      requestPreview({
        ...baseRequest,
        summary: "Summary",
        body_markdown: "Body",
      }),
    ).toBe("Summary");
  });

  it("falls back to the body as preview text when the summary is empty", () => {
    expect(
      requestPreview({
        ...baseRequest,
        summary: "",
        body_markdown: "Body",
      }),
    ).toBe("Body");
  });

  it("allows enrollment when required fields are present", () => {
    expect(
      canSubmitEnrollment({
        base_url: "https://nod.example.com",
        device_name: "Desktop",
        code: "ABCDEFGH",
      }),
    ).toBe(true);
  });

  it("blocks enrollment when the code is too short", () => {
    expect(
      canSubmitEnrollment({
        base_url: "https://nod.example.com",
        device_name: "Desktop",
        code: "ABC",
      }),
    ).toBe(false);
  });
});

describe("replaceRequest", () => {
  it("replaces the matching request in place and leaves the rest untouched", () => {
    const other: NodRequest = {
      ...baseRequest,
      id: "other",
      request_id: "other",
    };
    const resolved: NodRequest = { ...baseRequest, status: "resolved" };

    const next = replaceRequest([baseRequest, other], resolved);

    expect(next).toHaveLength(2);
    expect(next[0]).toBe(resolved);
    expect(next[1]).toBe(other);
  });

  it("drops an update whose id is not in the cache", () => {
    const stranger: NodRequest = {
      ...baseRequest,
      id: "stranger",
      request_id: "stranger",
    };

    const next = replaceRequest([baseRequest], stranger);

    expect(next).toHaveLength(1);
    expect(next[0]).toBe(baseRequest);
  });
});

describe("decisionActions", () => {
  const approveWithText: RequestOption = {
    ...baseOption,
    id: "approve_notes",
    label: "Approve with notes",
    kind: "approve_with_text",
  };
  const reject: RequestOption = {
    ...baseOption,
    id: "reject",
    label: "Reject",
    kind: "reject",
    destructive: true,
  };

  it("preserves every issuer option even when kinds are related", () => {
    const options = [
      baseOption,
      approveWithText,
      reject,
      { ...baseOption, id: "approve-later", label: "Approve later" },
    ];
    expect(
      decisionActions({ ...baseRequest, options }).map(
        (action) => action.option,
      ),
    ).toEqual(options);
  });

  it("keeps unpaired with-text and custom options standalone", () => {
    const custom: RequestOption = {
      ...baseOption,
      id: "later",
      kind: "custom",
    };
    const actions = decisionActions({
      ...baseRequest,
      options: [approveWithText, custom],
    });

    expect(actions).toHaveLength(2);
    expect(actions[0].option.id).toBe("approve_notes");
    expect(actions[0].withTextOption).toBeUndefined();
    expect(actions[1].option.id).toBe("later");
  });
});

describe("untrusted links", () => {
  it("accepts scoped enrollment without submitting it", () => {
    expect(
      parseEnrollmentLink(
        "nod://enroll?server=https%3A%2F%2Fnod.test%2F&code=abcdefgh",
      ),
    ).toEqual({ base_url: "https://nod.test/", code: "ABCDEFGH" });
  });
  it("rejects ambiguous enrollment servers and credential URLs", () => {
    expect(
      parseEnrollmentLink(
        "nod://enroll?server=https://a.test&server=https://b.test&code=ABCDEFGH",
      ),
    ).toBeUndefined();
    expect(safeWebUrl("https://trusted.test@evil.test/")).toBeUndefined();
    expect(safeWebUrl("javascript:alert(1)")).toBeUndefined();
  });
});
