import { Check, Copy, ExternalLink, X } from "lucide-react";
import { useRef, useState } from "react";
import { RequestImage } from "./RequestImage";
import Markdown from "react-markdown";
import { decisionActions, optionRequiresText, safeWebUrl } from "../domain";
import type { NodRequest, RequestOption } from "../types";

interface RequestDetailProps {
  request?: NodRequest;
  serverId?: string;
  allowRemoteImages?: boolean;
  onOption: (
    request: NodRequest,
    option: RequestOption,
    text?: string,
  ) => Promise<boolean>;
  onOpenUrl: (url: string) => Promise<void>;
}

export function RequestDetail({
  request,
  serverId = "",
  allowRemoteImages = false,
  onOption,
  onOpenUrl,
}: RequestDetailProps): JSX.Element {
  // Keep unsent text per server/request while navigating; never persist sensitive notes to disk.
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const inFlight = useRef(false);
  const [feedback, updateFeedback] = useState({ key: "", message: "" });
  const key = `${serverId}:${request?.id ?? ""}`;
  const setFeedback = (message: string): void =>
    updateFeedback({ key, message });
  const notes = drafts[key] ?? "";
  const trimmedNotes = notes.trim();
  if (!request)
    return (
      <section className="detail empty">Select a request to review</section>
    );
  const actions = decisionActions(request);
  const hasNotes = actions.some(({ option }) => optionRequiresText(option));
  const expired = request.expires_at
    ? Date.parse(request.expires_at) <= Date.now()
    : false;
  const image = request.image_url ? safeWebUrl(request.image_url) : undefined;
  const decisions = request.decisions.length
    ? request.decisions.map((entry) => entry.decision)
    : request.decision
      ? [request.decision]
      : [];

  async function submit(option: RequestOption): Promise<void> {
    if (!request || inFlight.current) return;
    inFlight.current = true;
    setBusy(key);
    setFeedback("");
    try {
      if (await onOption(request, option, trimmedNotes || undefined)) {
        setDrafts((current) => {
          const next = { ...current };
          delete next[key];
          return next;
        });
        setFeedback(`${option.label} recorded.`);
      } else {
        setFeedback(
          "Decision was not confirmed. Your notes are kept; refresh or retry.",
        );
      }
    } catch (error) {
      setFeedback(String(error));
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  }

  async function copyRequestId(): Promise<void> {
    try {
      await navigator.clipboard.writeText(request!.id);
      setFeedback("Request ID copied.");
    } catch {
      setFeedback(
        "Could not copy. Select the request ID below to copy it manually.",
      );
    }
  }

  return (
    <section
      className="detail"
      aria-label="Request details"
      aria-busy={busy === key}
    >
      <header>
        <span className={`pill ${request.status}`}>{request.status}</span>
        <h2>{request.title}</h2>
        <p>{request.summary}</p>
        <small>
          Channel: {request.channel_id} ·{" "}
          {request.decision_resolution === "per_user"
            ? "Individual decision"
            : "Shared decision"}{" "}
          · {new Date(request.created_at).toLocaleString()}
        </small>
        {request.expires_at ? (
          <p className={expired ? "formError" : "muted"}>
            {expired ? "Expired" : "Expires"}{" "}
            {new Date(request.expires_at).toLocaleString()}
          </p>
        ) : null}
      </header>
      {image ? (
        <div className="requestMedia">
          {allowRemoteImages ? (
            <RequestImage key={image} url={image} />
          ) : (
            <p>
              Remote image hidden.{" "}
              <button type="button" onClick={() => void onOpenUrl(image)}>
                Open attachment in browser
              </button>
            </p>
          )}
        </div>
      ) : null}
      {request.body_markdown ? (
        <div className="markdown">
          <Markdown
            skipHtml
            urlTransform={(url) => safeWebUrl(url) ?? ""}
            components={{
              a: ({ href, children }) =>
                href ? (
                  <button
                    type="button"
                    className="textLink"
                    title={href}
                    onClick={() => void onOpenUrl(href)}
                  >
                    {children}
                    <small>{href}</small>
                  </button>
                ) : (
                  <span>{children}</span>
                ),
              img: ({ alt }) => (
                <span className="muted">[Image: {alt || "attachment"}]</span>
              ),
            }}
          >
            {request.body_markdown}
          </Markdown>
        </div>
      ) : null}
      <dl>
        {request.fields.map((field, index) => (
          <div key={`${index}:${field.label}`}>
            <dt>{field.label}</dt>
            <dd>{field.value}</dd>
          </div>
        ))}
      </dl>
      <div className="links">
        {request.links.map((link, index) => (
          <button
            type="button"
            key={`${index}:${link.url}`}
            disabled={!safeWebUrl(link.url)}
            onClick={() => void onOpenUrl(link.url)}
          >
            <ExternalLink size={14} />
            {link.label}
            <small>{link.url}</small>
          </button>
        ))}
      </div>
      {request.status === "pending" ? (
        <div className="options">
          <div className="optionButtons">
            {actions.map(({ option }) => {
              const reject =
                option.destructive || option.kind.startsWith("reject");
              return (
                <button
                  type="button"
                  key={option.id}
                  className={reject ? "danger" : ""}
                  disabled={
                    busy !== null || expired
                  }
                  onClick={() => void submit(option)}
                >
                  {option.kind === "open" ? (
                    <ExternalLink size={16} />
                  ) : reject ? (
                    <X size={16} />
                  ) : option.kind.startsWith("approve") ? (
                    <Check size={16} />
                  ) : null}
                  {option.label}
                </button>
              );
            })}
          </div>
          {hasNotes ? (
            <label className="optionNotes">
              Notes (optional)
              <textarea
                value={notes}
                disabled={busy === key}
                onChange={(event) => {
                  const value = event.currentTarget.value;
                  setDrafts((current) => ({ ...current, [key]: value }));
                }}
                placeholder="Sent with whichever decision you pick"
                rows={3}
                maxLength={16384}
              />
            </label>
          ) : null}
          {busy === key ? <p role="status">Recording decision…</p> : null}
        </div>
      ) : null}
      {decisions.map((decision, index) => (
        <section className="receipt" key={`${index}:${decision.resolved_at}`}>
          <h3>{decision.option_label}</h3>
          <p>{decision.text}</p>
          <small>
            {new Date(decision.resolved_at).toLocaleString()} ·{" "}
            {decision.actor_user_id ?? "Unknown user"} · Device{" "}
            {decision.actor_device_id ?? "unknown"}
          </small>
          <p>
            {decision.signature?.verified
              ? "Signature verified by server"
              : "No verified signature receipt"}
          </p>
        </section>
      ))}
      <footer className="requestIdentity">
        <code>{request.id}</code>
        <button
          type="button"
          aria-label="Copy request ID"
          onClick={() => void copyRequestId()}
        >
          <Copy size={14} />
        </button>
      </footer>
      <p role="status" className="muted">
        {feedback.key === key ? feedback.message : ""}
      </p>
    </section>
  );
}
