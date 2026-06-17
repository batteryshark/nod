<p align="center">
  <img src="assets/nod-icon.png" width="180" alt="Nod">
</p>

# Nod

Nod is a self-hosted approval layer for personal agents, automations, and
services. When software needs an external decision, from a person or an
agentic evaluator in the loop, it can create a Nod request, fan it out to the
right recipients and devices, wait for an answer, and receive a durable
decision record.

The goal is to make "route this through a decision-maker before doing it" a
first-class protocol rather than a pile of one-off push notifications, chat
messages, and scripts.

## Quickstart

The server is one self-contained binary — it runs on a laptop. Grab it from
the [latest release](https://github.com/batteryshark/nod/releases/latest):

```bash
tar -xzf nod-server-*.tar.gz
NOD_ADMIN_TOKEN=$(openssl rand -hex 24) ./nod-server
```

Open `http://localhost:8767/admin`, log in with the token, mint an enrollment
code, and connect a client. Full per-OS instructions (including Windows,
Docker, run-at-login, and remote access over Tailscale) are in
[docs/deploy.md](docs/deploy.md).

## Clients

| Client | Platforms | Install |
| --- | --- | --- |
| Native app (`client/nod-apple`) | macOS | Notarized DMG on the release page |
| Native app (`client/nod-apple`) | iPhone / iPad | TestFlight |
| Desktop app (`client/nod-desktop`) | Windows | Zipped `Nod.exe` on the release page — put it anywhere (unsigned; see SmartScreen note in [docs/deploy.md](docs/deploy.md)) |
| Terminal UI (`client/nod-tui`) | macOS / Linux / Windows | Binary on the release page |

Every client enrolls the same way: server URL + enrollment code. Decisions are
signed on-device — Secure Enclave on Apple hardware, software P-256 keys
elsewhere.

| macOS | Windows | Terminal |
| --- | --- | --- |
| ![Nod on macOS: handled requests for a channel](assets/macos-desktop.png) | ![Nod on Windows: pending approval with notes](assets/windows-desktop.png) | ![Nod TUI: servers, channels, and request detail](assets/tui-desktop.png) |

## Why Not Just Use Chat?

Messaging services are built for conversation. Nod is built for decisions.

In chat, an approval is usually an unstructured message in a busy room. It is
hard to know who was eligible to answer, which device received the prompt,
whether the answer matched the exact request snapshot, whether someone replied
after a timeout or cancellation, and how an automation should consume the
result.

Nod makes those ideas explicit:

- channels define scopes that users can subscribe to without turning every
  request into a noisy chat thread
- requests carry structured context and action options instead of relying on
  free-form replies
- shared requests can be resolved by the first eligible user who approves,
  rejects, or dismisses
- per-user requests collect each recipient's own decision so issuers can apply
  quorum, consensus, or audit policies on top
- decisions can include signed text reasons and machine-readable option ids
- every result is available over read/wait APIs and durable audit records

## What It Does

- Agents and services create decision requests with issuer tokens.
- Admins manage users, channels, issuer tokens, and short-lived enrollment codes.
- Clients enroll devices against a server, and the same client can be connected
  to multiple Nod servers.
- A request can target one user, several users, or every subscribed user for a
  channel.
- Each enrolled device for those users is notified over WebSocket/local
  notifications or full Apple Push Notification service delivery through the
  APNs relay.
- Users can approve, reject, dismiss, open, or choose custom actions. Approval
  and rejection options can require a text reason that is returned to the
  issuer.
- Issuers can read decisions, wait for a result, cancel pending requests, set
  timeouts, dedupe retried creates, and optionally receive callback wake-ups.

## Request Model

Nod requests are structured cards, not just strings. A request can include:

- title, summary, and Markdown body
- structured fields for values like environment, amount, risk, or owner
- links to runbooks, dashboards, diffs, tickets, or logs
- optional image URL for screenshots or other context
- priority, privacy, dedupe key, expiry, and optional callback wake-up URL
- shared resolution, where one decision resolves the request for everyone
- per-user resolution, where each recipient makes their own decision
- options such as `approve`, `approve_with_text`, `reject`,
  `reject_with_text`, `dismiss`, `open`, and `custom`

Example:

```json
{
  "channel_id": "deployments",
  "recipients": ["owner", "platform"],
  "decision_resolution": "shared",
  "title": "Approve production deploy",
  "summary": "api-gateway v42 is ready for production",
  "body_markdown": "**Change:** roll forward api-gateway to v42.\n\nCanary is green and error budget impact is low.",
  "fields": [
    { "label": "Service", "value": "api-gateway" },
    { "label": "Environment", "value": "production" },
    { "label": "Risk", "value": "medium", "style": "warning" }
  ],
  "links": [
    { "label": "Diff", "url": "https://example.com/diff/42" },
    { "label": "Runbook", "url": "https://example.com/runbooks/deploy" }
  ],
  "image_url": "https://example.com/screenshots/canary.png",
  "priority": 8,
  "dedupe_key": "deploy:api-gateway:42",
  "expires_at": "2027-01-01T00:10:00Z",
  "options": [
    { "id": "approve", "label": "Approve", "kind": "approve" },
    {
      "id": "approve_notes",
      "label": "Approve with notes",
      "kind": "approve_with_text",
      "text_placeholder": "Reason or constraint"
    },
    {
      "id": "reject_reason",
      "label": "Reject with reason",
      "kind": "reject_with_text",
      "destructive": true
    }
  ]
}
```

Requests with no explicit options still support a lightweight `dismiss` flow,
which is useful for notification-style prompts where the issuer only needs to
know that the user saw and cleared the item.

## Security And Delivery

Nod is designed for private, self-hosted deployments. The main server stores
request state, device records, signed decisions, and append-only audit logs.
Devices can register P-256 signing keys during enrollment; decision submissions
can then be signed against the server-provided request digest, with nonce reuse
rejected per device key.

The issuer read and wait APIs are the authoritative way to consume decisions.
If a request sets `callback_url`, Nod sends an unsigned, best-effort POST after
a decision is recorded. Treat that callback as a wake-up hint only: receivers
must authenticate their own route and read or wait for the decision before
acting on an approval.

Apple push is handled by a separate mTLS APNs relay. That keeps Apple provider
credentials out of the main server while still allowing iOS and macOS clients to
receive background and lock-screen push notifications. Without a configured
relay, clients can still use WebSocket sync and local notifications while they
are connected.

## Monorepo Layout

- `server/nod-server`: self-hosted Rust decision server, admin panel, sync API,
  audit log, and optional remote-push integration.
- `server/nod-apns-relay`: standalone mTLS APNs relay that keeps Apple push
  credentials outside the main server.
- `client/nod-apple`: native SwiftUI macOS and iOS clients.
- `client/nod-client-core`: Rust client runtime shared by desktop and TUI
  clients.
- `client/nod-desktop`: Tauri + React desktop client for Windows and Linux.
- `client/nod-tui`: Ratatui terminal client for headless workflows.

Each project directory has its own README with build, test, and deployment
details.

## Development

One Cargo workspace covers the server, relay, and every Rust client:

```bash
cargo test --workspace
```

The relay's TLS tests need local fixtures once:
`server/nod-apns-relay/tests/fixtures/mtls/generate`. The full pre-release
gate (Swift, desktop frontend, drift check, end-to-end smoke) lives in
[docs/release-checklist.md](docs/release-checklist.md), and
`server/nod-server/scripts/nod-smoke` exercises a running server end to end.

## License

[AGPL-3.0](LICENSE). Self-host it freely; if you run a modified Nod as a
service for others, share your changes.
