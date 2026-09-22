# Nod Desktop

Tauri 2 desktop client for Nod. Windows is the shipped platform — a zipped,
unsigned `Nod.exe` users place wherever they want (no installer; Linux bundles
are configured but not yet released). It runs fine on macOS for
development, but macOS users get the native app in `client/nod-apple` instead —
that one signs decisions with the Secure Enclave.

## Development

Use Node.js 22.12 or newer and Rust stable. Development works on macOS, Windows,
or Linux (native desktop dependencies are required on Linux):

```bash
npm ci
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

The release workflow ships the bare exe (`npm run tauri build -- --no-bundle`)
zipped; `windows-exe.yml` builds the same artifact on demand for VM testing,
and `scripts/build-windows-exe` cross-compiles it locally from macOS.

## Test

```bash
npm run typecheck
npm test
npm run drift-check
cargo test --manifest-path src-tauri/Cargo.toml
```

`drift-check` regenerates the typeshare projection (`src/dto/generated.ts`,
git-ignored) from the Rust `#[typeshare]` types and compares it against the
hand-written contract in `src/dto/models.ts` — names must agree; deliberate
divergences are documented in `scripts/check-drift.mjs`. It needs
`cargo install typeshare-cli --version 1.13.4 --locked` once.

The frontend talks to the Rust runtime through typed wrappers in `src/commands.ts`.
Do not call Tauri `invoke` directly from React components.


## Inbox and notifications

All channels shows the active server's eligible requests. Channel changes filter
that cache immediately. Search, status and date filters apply to the loaded
inbox; **Search server history** fetches older or cleared items with pagination.
Clearing hides handled requests for your account without deleting them or hiding
pending work. Arrow keys move between requests while a request card has focus.

Details show the original option labels, Markdown, link destinations, decision
receipts and a copyable request ID. Notes are optional even when an option opens
a text field. Failed submissions keep your draft in memory for that server and
request; drafts are not saved to disk. If delivery is uncertain, refresh before
retrying to see whether a decision was recorded.

Settings offers launch at login, notification sound, hidden previews, per-channel
mute, one-hour snooze and local quiet hours. These controls do not change channel
eligibility or approve requests. A test notification reports acceptance by the
local OS; Focus/Do Not Disturb or OS permissions may still prevent presentation.
Native Windows/Linux actions fetch the original server/request before acting.
Redacted or oversized option sets open the request instead of exposing or hiding
individual choices. Terminal updates remove obsolete notifications.

Remote images are off by default. Opt-in PNG/JPEG previews have a 12-second,
8 MiB, 4096-pixel-per-axis decode limit and at most four concurrent operations;
the webview receives a static thumbnail. Remote URLs contact the image host;
redirects and inline Markdown images are not loaded. Other attachments can be
opened in the browser. The bundled webview CSP allows local assets and IPC only.

Native Windows code runs in the Windows CI job. macOS tests cannot exercise
Windows toast activation, history removal or ShellExecute; those still need a
Windows acceptance pass before release. Tauri's macOS shell is for development;
use the native Apple client for supported macOS notifications.
