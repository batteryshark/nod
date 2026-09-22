import { HistorySearch } from "./HistorySearch";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useEffect, useState } from "react";
import { requestPreview, orderedRequests } from "../domain";
import type { NodRequest, RequestStatus } from "../types";

interface RequestListProps {
  requests: NodRequest[];
  serverId: string;
  channelId?: string;
  onSelect: (request: NodRequest) => Promise<void>;
  selectedRequestId: string | null;
}

// Mirrors the macOS inbox: Pending opens expanded, Handled stays tucked away
// until there is nothing left to act on. The parent keys this component by
// channel, so switching channels resets the expansion state.
export function RequestList({
  requests,
  serverId,
  channelId,
  onSelect,
  selectedRequestId,
}: RequestListProps): JSX.Element {
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<RequestStatus | "all">("all");
  const [since, setSince] = useState("");
  const term = search.trim().toLocaleLowerCase();
  const ordered = orderedRequests(requests).filter(
    (request) =>
      (status === "all" || request.status === status) &&
      (!since || request.created_at.slice(0, 10) >= since) &&
      (!term ||
        [
          request.title,
          request.summary,
          request.body_markdown,
          request.id,
          ...request.fields.map((field) => field.value),
        ].some((text) => text.toLocaleLowerCase().includes(term))),
  );
  const pending = ordered.filter((request) => request.status === "pending");
  const handled = ordered.filter((request) => request.status !== "pending");
  const [pendingExpanded, setPendingExpanded] = useState(true);
  const [handledExpanded, setHandledExpanded] = useState(false);

  const hasPending = pending.length > 0;
  useEffect(() => {
    if (!hasPending) {
      setHandledExpanded(true);
    }
  }, [hasPending]);

  function card(request: NodRequest): JSX.Element {
    return (
      <button
        type="button"
        key={request.id}
        aria-pressed={request.id === selectedRequestId}
        className={
          request.id === selectedRequestId
            ? "requestCard active"
            : "requestCard"
        }
        onClick={() => void onSelect(request)}
      >
        <span className={`status ${request.status}`} />
        <strong>{request.title}</strong>
        <span>
          {request.channel_id} · {requestPreview(request)}
        </span>
        <time>{new Date(request.created_at).toLocaleString()}</time>
      </button>
    );
  }

  return (
    <section
      className="requestList"
      aria-label="Requests"
      onKeyDown={(event) => {
        if (
          !["ArrowDown", "ArrowUp"].includes(event.key) ||
          !(event.target instanceof HTMLButtonElement) ||
          !event.target.classList.contains("requestCard")
        )
          return;
        event.preventDefault();
        const index = ordered.findIndex(
          (request) => request.id === selectedRequestId,
        );
        const next =
          ordered[
            Math.max(
              0,
              Math.min(
                ordered.length - 1,
                index + (event.key === "ArrowDown" ? 1 : -1),
              ),
            )
          ];
        if (next) void onSelect(next);
      }}
    >
      <div className="inboxFilters">
        <label>
          Search
          <input
            type="search"
            value={search}
            placeholder="Title, content or request ID"
            onChange={(event) => setSearch(event.currentTarget.value)}
          />
        </label>
        <label>
          Status
          <select
            value={status}
            onChange={(event) =>
              setStatus(event.currentTarget.value as RequestStatus | "all")
            }
          >
            <option value="all">All</option>
            <option value="pending">Pending</option>
            <option value="resolved">Resolved</option>
            <option value="expired">Expired</option>
            <option value="cancelled">Cancelled</option>
          </select>
        </label>
        <label>
          Since
          <input
            type="date"
            value={since}
            onChange={(event) => setSince(event.currentTarget.value)}
          />
        </label>
      </div>
      {ordered.length === 0 ? <p className="empty">No Requests</p> : null}
      {pending.length > 0 ? (
        <>
          <SectionHeader
            title="Pending"
            count={pending.length}
            expanded={pendingExpanded}
            onToggle={() => setPendingExpanded((expanded) => !expanded)}
          />
          {pendingExpanded ? pending.map(card) : null}
        </>
      ) : null}
      {handled.length > 0 ? (
        <>
          <SectionHeader
            title="Handled"
            count={handled.length}
            expanded={handledExpanded}
            onToggle={() => setHandledExpanded((expanded) => !expanded)}
          />
          {handledExpanded || term || status !== "all" || since
            ? handled.map(card)
            : null}
        </>
      ) : null}
      <HistorySearch
        serverId={serverId}
        channelId={channelId}
        onSelect={onSelect}
      />
    </section>
  );
}

interface SectionHeaderProps {
  title: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
}

function SectionHeader({
  title,
  count,
  expanded,
  onToggle,
}: SectionHeaderProps): JSX.Element {
  return (
    <button
      type="button"
      className="sectionHeader"
      aria-expanded={expanded}
      onClick={onToggle}
    >
      {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
      {title}
      <span className="sectionCount">{count}</span>
    </button>
  );
}
