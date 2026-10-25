# Changelog

## Unreleased

- iOS: the connection status is a navigation-bar indicator instead of a bottom
  bar that covered the tab bar and request controls.

## v1.2.0

- Security: every client signs the digest of the request the user saw; a
  signing key is mandatory at enrollment and every device decision carries a
  verified signature; no approve/reject from notifications on any platform.
- `nod-verify` CLI and `nod_proto::verify_decision_receipt` check a receipt
  offline against device keys pinned from the admin device list, which now
  exposes `signing_key_id` and `signing_public_key`.
- The sync WebSocket authenticates with the `Authorization` header only; 1.1.x
  clients get 401 and must be updated with the server.
- Admin cookie `Secure` behind TLS, delayed failed admin logins, uniform
  enrollment codes, http(s)-only link/image URLs, 0700/0600 data and audit files.
- Request digests collapsed: ordinary cards issue `nod-request-v1` with
  `nod-decision-v1`, value cards `nod-request-v2` with `nod-decision-v2`;
  older encoders stay verification-only and already-issued cards keep working.
- New Nod mascot app icon across iOS, macOS and the desktop app, with a
  matching menu-bar and tray glyph (the unpublished 1.1.1 refresh).

Full notes: [docs/release-notes/v1.2.0.md](docs/release-notes/v1.2.0.md)

## v1.1.0

- Committed lifecycle outbox with optional NATS publishing, durable callback
  retries, leased push recovery, and live-update resynchronization.
- Desktop-first Inset admin, searchable revoked-token log and bundled emoji
  picker; fixes for select-subscribed, logout cleanup and partial membership saves.
- Inset Inbox for iOS and macOS, with scrollable access to every answer option.
- One optional defaulted choice, number, date or UTC-minute answer value plus
  notes, bound into a versioned signed decision; ordinary signing bytes preserved.
- TUI choice editing and safe updated-app guidance for unsupported value cards.

- macOS recovers from transient gateway errors on safe GET requests, coalesces
  foreground refreshes, and keeps Dock badges updated without replaying decisions.
- macOS Pending Only clears filtered-out details; informational requests stay
  pending while reading and acknowledge only on explicit navigation away when enabled.
- Apple deployment docs explain direct ad-hoc installation and signing renewal
  without uninstalling or losing enrollment.

Full notes: [docs/release-notes/v1.1.0.md](docs/release-notes/v1.1.0.md)

## v1.0.1 - maintenance release

- APNs relay provider tokens are cached and reused so Apple no longer rejects
  bursts of pushes with `429 TooManyProviderTokenUpdates`.
- macOS keeps realtime sync serviced while Nod lives in the menu bar, so new
  requests can arrive without refocusing the app.
- README now includes macOS, Windows, and TUI screenshots from the v1 release
  verification pass.

Full notes: [docs/release-notes/v1.0.1.md](docs/release-notes/v1.0.1.md)

## v1.0.0 — first public release

- Self-hosted decision server: structured requests, channels, multi-user /
  multi-device enrollment, shared and per-user resolution, expiration,
  cancellation, dedupe, callbacks, audit logs, embedded admin panel.
- Decisions signed on-device with P-256 keys (Secure Enclave on Apple
  hardware), verified against the immutable request snapshot, with nonce
  replay rejection.
- Clients: native macOS app (notarized DMG), iOS via TestFlight, Windows
  desktop app (Tauri, zipped unsigned exe), terminal UI.
- Apple push via an in-process APNs route or a credential-isolating mTLS
  relay; WebSocket sync + local notifications everywhere else.
- Self-contained server binaries for macOS/Linux/Windows and a Docker image
  on GHCR.

Full notes: [docs/release-notes/v1.0.0.md](docs/release-notes/v1.0.0.md)
