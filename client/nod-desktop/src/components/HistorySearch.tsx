import { useEffect, useRef, useState } from "react";
import { queryHistory } from "../commands";
import type { NodRequest } from "../types";
interface Props {
  serverId: string;
  channelId?: string;
  onSelect: (request: NodRequest) => Promise<void>;
}
export function HistorySearch({
  serverId,
  channelId,
  onSelect,
}: Props): JSX.Element {
  const [search, setSearch] = useState("");
  const [result, setResult] = useState<NodRequest[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);
  const generation = useRef(0);
  const committedSearch = useRef("");
  useEffect(
    () => () => {
      generation.current += 1;
    },
    [],
  );
  async function load(more: boolean): Promise<void> {
    if (busy) return;
    const current = ++generation.current;
    setBusy(true);
    setError("");
    if (!more) committedSearch.current = search;
    try {
      const page = await queryHistory({
        server_id: serverId,
        channel_id: channelId,
        search: committedSearch.current || undefined,
        before: more ? (cursor ?? undefined) : undefined,
        limit: 25,
      });
      if (generation.current !== current) return;
      setResult((old) =>
        more
          ? [
              ...old,
              ...page.requests.filter(
                (request) => !old.some((item) => item.id === request.id),
              ),
            ]
          : page.requests,
      );
      setCursor(page.next_cursor ?? null);
      setLoaded(true);
    } catch (reason) {
      if (generation.current === current) setError(String(reason));
    } finally {
      if (generation.current === current) setBusy(false);
    }
  }
  return (
    <details className="historySearch">
      <summary>Search server history</summary>
      <p className="muted">
        Includes handled requests hidden from your inbox, within server
        retention.
      </p>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void load(false);
        }}
      >
        <label>
          History search
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            placeholder="Search or leave blank for recent history"
          />
        </label>
        <button type="submit" disabled={busy}>
          Search history
        </button>
      </form>
      {error ? <p role="alert">{error}</p> : null}
      {busy ? <p role="status">Loading history…</p> : null}
      {loaded && !result.length && !busy ? <p>No matching history.</p> : null}
      {result.map((request) => (
        <button
          className="requestCard"
          type="button"
          key={request.id}
          onClick={() => void onSelect(request)}
        >
          <span className={`status ${request.status}`} />
          <strong>{request.title}</strong>
          <span>
            {request.status} · {request.channel_id}
          </span>
          <time>{new Date(request.created_at).toLocaleString()}</time>
        </button>
      ))}
      {cursor ? (
        <button type="button" disabled={busy} onClick={() => void load(true)}>
          Load older requests
        </button>
      ) : null}
    </details>
  );
}
