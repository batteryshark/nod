import type {
  ClientState,
  EnrollParams,
  NodRequest,
  RequestOption,
  RequestStatus,
  Channel,
  AnswerValue,
} from "./types";

// Mirror nod-proto's request-digest tags in one UI-layer place. Values are the
// canonical constants from nod-proto/src/signing.rs and value_signing.rs.
export const REQUEST_SIGNING_VERSIONS = {
  ordinary: "nod-request-v1",
  legacyOrdinary: "nod-request-v2",
  value: "nod-request-v2",
  legacyValue: "nod-request-v3",
} as const;

const statusRank: Record<RequestStatus, number> = {
  pending: 0,
  resolved: 1,
  expired: 1,
  cancelled: 1,
};

const defaultDismissOption: RequestOption = {
  id: "dismiss",
  label: "Dismiss",
  kind: "dismiss",
  style: "default",
  requires_text: false,
  destructive: false,
  foreground: false,
};

export function totalPendingCount(state: ClientState): number {
  return Object.values(state.pending_counts_by_channel).reduce(
    (total, count) => total + count,
    0,
  );
}

export function pendingCountFor(channel: Channel, state: ClientState): number {
  return state.pending_counts_by_channel[channel.id] ?? 0;
}

export function channelColor(channel: Channel): string {
  const palette = [
    "#2563EB",
    "#059669",
    "#D97706",
    "#DC2626",
    "#7C3AED",
    "#0891B2",
    "#4F46E5",
    "#DB2777",
  ];
  let hash = 0;
  for (const char of channel.id) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return palette[hash % palette.length];
}

export function orderedRequests(requests: readonly NodRequest[]): NodRequest[] {
  return [...requests].sort((left, right) => {
    const rankDelta = statusRank[left.status] - statusRank[right.status];
    if (rankDelta !== 0) {
      return rankDelta;
    }
    const timeDelta =
      new Date(right.created_at).getTime() -
      new Date(left.created_at).getTime();
    if (timeDelta !== 0) {
      return timeDelta;
    }
    return right.id.localeCompare(left.id);
  });
}

// The decision result replaces its request in place — by id, other entries
// untouched, and an id the cache doesn't know is dropped rather than appended
// (a submit always targets a cached request; growth comes from state syncs).
export function replaceRequest(
  requests: readonly NodRequest[],
  updated: NodRequest,
): NodRequest[] {
  return requests.map((candidate) =>
    candidate.id === updated.id ? updated : candidate,
  );
}

export function selectedRequest(state: ClientState): NodRequest | undefined {
  return (
    state.requests.find(
      (request) => request.id === state.selected_request_id,
    ) ?? orderedRequests(state.requests)[0]
  );
}

export function selectedChannel(state: ClientState): Channel | undefined {
  return state.channels.find(
    (channel) => channel.id === state.selected_channel_id,
  );
}

export function submittableOptions(request: NodRequest): RequestOption[] {
  if (isValueUnsupported(request)) return [];
  return request.options.length === 0
    ? [defaultDismissOption]
    : request.options;
}

export const VALUE_UNSUPPORTED_INSTRUCTION =
  "Open this request in an updated iOS or macOS Nod app to answer it.";

export function receiptValue(value: AnswerValue): string {
  switch (value.kind) {
    case "choice": return value.choice_id;
    case "number": return value.decimal;
    case "date": return value.date;
    case "date_time": return value.instant;
  }
}

export function isValueUnsupported(request: NodRequest): boolean {
  return request.answer_field != null ||
    request.signing?.version === REQUEST_SIGNING_VERSIONS.legacyValue ||
    request.options.some((option) => option.id === "__nod_value_open");
}

export function optionRequiresText(option: RequestOption): boolean {
  return option.requires_text || option.kind.endsWith("_with_text");
}

export interface DecisionAction {
  /** Submitted when the notes field is empty. */
  option: RequestOption;
  /** Submitted instead when notes are filled in, so the issuer sees the with-text option. */
  withTextOption?: RequestOption;
}

// Option IDs and labels are issuer-defined contracts, even when kinds match.
export function decisionActions(request: NodRequest): DecisionAction[] {
  return submittableOptions(request).map((option) => ({ option }));
}

export function safeWebUrl(value: string): string | undefined {
  try {
    const url = new URL(value);
    if (
      !["https:", "http:"].includes(url.protocol) ||
      !url.hostname ||
      url.username ||
      url.password
    )
      return undefined;
    return url.href;
  } catch {
    return undefined;
  }
}

export function parseEnrollmentLink(
  value: string,
): Pick<EnrollParams, "base_url" | "code"> | undefined {
  try {
    const url = new URL(value);
    if (
      url.protocol !== "nod:" ||
      url.hostname !== "enroll" ||
      url.username ||
      url.password
    )
      return undefined;
    if (
      url.searchParams.getAll("server").length !== 1 ||
      url.searchParams.getAll("code").length !== 1
    )
      return undefined;
    const base_url = safeWebUrl(url.searchParams.get("server") ?? "");
    const code = url.searchParams.get("code") ?? "";
    return base_url && /^[a-z0-9]{8}$/i.test(code)
      ? { base_url, code: code.toUpperCase() }
      : undefined;
  } catch {
    return undefined;
  }
}

export function requestPreview(request: NodRequest): string {
  return request.summary || request.body_markdown;
}

export function canSubmitEnrollment(
  draft: Pick<EnrollParams, "base_url" | "device_name" | "code">,
): boolean {
  return (
    safeWebUrl(draft.base_url.trim()) !== undefined &&
    draft.device_name.trim().length > 0 &&
    /^[a-z0-9]{8}$/i.test(draft.code.trim())
  );
}
