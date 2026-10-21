# Changelog

## Unreleased

- macOS recovers from transient gateway errors on safe GET requests, coalesces
  foreground refreshes, and keeps Dock badges updated without replaying decisions.
- macOS Pending Only clears filtered-out details; informational requests stay
  pending while reading and acknowledge only on explicit navigation away when enabled.
- Apple deployment docs explain direct ad-hoc installation and signing renewal
  without uninstalling or losing enrollment.

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
