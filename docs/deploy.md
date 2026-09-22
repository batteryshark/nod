# Run Nod on your own machine

The Nod server is one self-contained binary with an embedded admin panel and
a SQLite file for state. It runs fine on a laptop — a MacBook, a Windows
machine, or any Linux box. No database server, no Docker required.

## Path A — prebuilt binary (recommended)

Download the server archive for your machine from the
[latest release](https://github.com/batteryshark/nod/releases/latest):

| Machine | Archive |
| --- | --- |
| Mac (Apple Silicon) | `nod-server-v1.0.0-aarch64-apple-darwin.tar.gz` |
| Mac (Intel) | `nod-server-v1.0.0-x86_64-apple-darwin.tar.gz` |
| Linux (x86_64) | `nod-server-v1.0.0-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (ARM64) | `nod-server-v1.0.0-aarch64-unknown-linux-gnu.tar.gz` |
| Windows | `nod-server-v1.0.0-x86_64-pc-windows-msvc.zip` |

### macOS / Linux

```bash
tar -xzf nod-server-*.tar.gz
mkdir -p ~/nod && cd ~/nod
NOD_ADMIN_TOKEN=$(openssl rand -hex 24)
echo "$NOD_ADMIN_TOKEN" > admin-token.txt && chmod 600 admin-token.txt
NOD_ADMIN_TOKEN="$NOD_ADMIN_TOKEN" /path/to/nod-server
```

macOS note: the binaries are unsigned, so the first run may be blocked by
Gatekeeper. Clear it with `xattr -d com.apple.quarantine /path/to/nod-server`
(or right-click → Open once).

### Windows (PowerShell)

```powershell
Expand-Archive nod-server-*-x86_64-pc-windows-msvc.zip -DestinationPath $HOME\nod
cd $HOME\nod
$env:NOD_ADMIN_TOKEN = -join ((1..48) | ForEach-Object { '{0:x}' -f (Get-Random -Max 16) })
$env:NOD_ADMIN_TOKEN | Out-File -Encoding ascii admin-token.txt
.\nod-server.exe
```

SmartScreen note: downloaded unsigned executables show "Windows protected
your PC" on first run — choose **More info → Run anyway**. Verify the
download first: `Get-FileHash nod-server-*.zip` must match the line in the
release's `SHA256SUMS` file.

### First five minutes

1. Open `http://localhost:8767/admin` and log in with your admin token.
2. Mint an **enrollment code** for your user (a `default` channel and an
   `owner` user are seeded for you; add per-tool channels as you grow).
3. Open a Nod client (macOS app, Windows app, iOS via TestFlight, or
   `nod-tui` in a terminal), point it at `http://localhost:8767`, and enter
   the code.
4. In the admin panel, create an **issuer token** for your agent or script,
   then send your first request:

   ```bash
   curl -X POST http://localhost:8767/api/v1/requests \
     -H "Authorization: Bearer <issuer-token>" \
     -H "Content-Type: application/json" \
     -d '{"channel_id":"default","title":"Deploy to prod?","summary":"v1.0.0 is ready","options":[{"id":"approve","label":"Ship it","kind":"approve"},{"id":"reject","label":"Hold","kind":"reject"}]}'
   ```

   The request reaches enrolled devices of subscribed users. Current clients
   sign decisions with their enrolled device key.

State lives in `.nod/` next to where you started the server (override with
`NOD_DATA_DIR` and `NOD_DATABASE_URL`). Back it up by copying that directory
while the server is stopped.

### Run at login (optional)

<details>
<summary>macOS — launchd</summary>

Save as `~/Library/LaunchAgents/com.nod.server.plist`, adjust the two paths,
then `launchctl load ~/Library/LaunchAgents/com.nod.server.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.nod.server</string>
  <key>ProgramArguments</key>
  <array><string>/Users/you/nod/nod-server</string></array>
  <key>WorkingDirectory</key><string>/Users/you/nod</string>
  <key>EnvironmentVariables</key>
  <dict><key>NOD_ADMIN_TOKEN_FILE</key><string>/Users/you/nod/admin-token.txt</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
```

</details>

<details>
<summary>Linux — systemd</summary>

Save as `/etc/systemd/system/nod.service`, then
`systemctl enable --now nod`:

```ini
[Unit]
Description=Nod server
After=network.target

[Service]
User=nod
WorkingDirectory=/home/nod
ExecStart=/home/nod/nod-server
Environment=NOD_ADMIN_TOKEN_FILE=/home/nod/admin-token.txt
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

</details>

<details>
<summary>Windows — Task Scheduler</summary>

Task Scheduler → Create Task → trigger **At log on** → action
`C:\Users\you\nod\nod-server.exe`, start in `C:\Users\you\nod`. Set
`NOD_ADMIN_TOKEN_FILE` as a user environment variable pointing at your
`admin-token.txt`.

</details>

## Path B — Docker

```bash
docker run -d --name nod \
  -e NOD_ADMIN_TOKEN=change-me \
  -p 127.0.0.1:8767:8767 \
  -v nod-data:/data \
  ghcr.io/batteryshark/nod-server:latest
```

For Compose (auto-generated admin token, optional APNs relay), use the files
in [`server/nod-server/`](../server/nod-server/README.md#docker-compose):

```bash
server/nod-server/scripts/nod-compose up -d --build
```

## Reaching it from your phone or another machine

The server speaks plain HTTP on `8767` and is meant to stay private — don't
expose it directly to the internet.

- **Tailscale** (easiest): with the server machine on your tailnet,
  `tailscale serve --bg --set-path /nod 8767` gives every one of your devices
  an HTTPS URL like `https://your-machine.your-tailnet.ts.net/nod`. Use that
  URL when enrolling clients.
- **Reverse proxy**: terminate TLS in nginx/Caddy/Traefik and proxy to
  `127.0.0.1:8767` (WebSocket upgrade required for `/api/v1/sync`).

## Verifying a deployment

```bash
curl -fsS https://your-nod-host/health
server/nod-server/scripts/nod-smoke https://your-nod-host "$NOD_ADMIN_TOKEN"
```

The smoke run enrolls a throwaway device, pushes a request through the sync
WebSocket, submits a signed decision, and removes everything it created.

## Advanced: push notifications for iPhone/iPad

Everything above works without Apple Push. The macOS app, Windows app, and TUI
receive requests over the sync WebSocket, and the iOS app can refresh and
receive WebSocket updates while it is running. Background and lock-screen iOS
delivery still require APNs; there is no native iOS push path that bypasses
Apple's push service.

There are two honest iOS shapes:

| Shape | What you run | Tradeoff |
| --- | --- | --- |
| Official Nod iOS app + project-operated push relay | Your own Nod server, with iOS background push routed through the project's APNs relay | Easiest for users, but not fully self-hosted because push delivery depends on the project relay |
| Fully self-hosted iOS | Your Nod server plus your own Apple Developer Program account, bundle id/App ID with push enabled, APNs auth key or certificate, and a production TestFlight/App Store build distributed under your Apple team | More setup, but the APNs credentials and app distribution are yours |

For the fully self-hosted route, configure the matching
`NOD_APNS_DIRECT_*` settings or the mTLS relay-backed `NOD_APNS_RELAY_*`
settings. See
[APNs configuration](../server/nod-server/README.md#push-providers) for the
direct and relay options.

## Privacy model in v1

Nod v1.0.0 is a self-hostable ownership release: it gives you the server,
clients, and release artifacts needed to own the decision loop today. It is
not yet a privacy-preserving hosted-service design. The server operator can
see request content, options, recipients, decisions, timestamps, delivery
state, callback URLs, callback audit logs, and server logs.

Future private-push-relay and content-private-server modes could make limited
centralization safer for families, companies, and friend groups that do not
want the operator reading request contents. In those modes, a relay would send
generic or opaque notifications, and request bodies/options could be encrypted
for recipient devices while the server still sees routing metadata such as
recipients, channels, timestamps, and delivery state.

## Back up and restore a server

Stop Nod before taking a complete backup so the SQLite state and audit files
represent the same point in time. Stop its service manager or container as well
so it does not immediately restart. A copy of only `nod.sqlite` from a running
server can omit transactions still in its WAL file.

For the default binary deployment, from the directory where Nod runs:

```bash
# After stopping Nod and confirming its process has exited:
umask 077
backup_dir="nod-backup-$(date +%Y%m%d-%H%M%S)"
mkdir "$backup_dir"
cp -R .nod "$backup_dir/data"
cp admin-token.txt "$backup_dir/admin-token.txt"
# The repository README instead creates nod-data/admin-token; copy that file
# when it is your configured NOD_ADMIN_TOKEN_FILE.
```

If configured, also copy the TOML configuration, secret files, and the SQLite
file named by `NOD_DATABASE_URL` when it is outside `NOD_DATA_DIR`. Keep the
complete audit directory, including rotated `nod.audit.*.jsonl` archives.
Include any SQLite `-wal` and `-shm` files with a stopped database copy. For a
container, stop it and copy or snapshot the mounted `/data` volume; the
container image alone is not a backup. Protect backups as secrets: the database
contains request content, token hashes, device public keys, signature evidence,
and enrollment state; the audit files contain decisions and callback diagnostics.

To restore:

1. Keep the old data untouched and extract the backup into a new directory.
2. With Nod stopped, point `NOD_DATA_DIR` and `NOD_DATABASE_URL` at the restored
   data, restore the configuration and protected secret files, and give the Nod
   service account read/write access. Do not overlay a backup onto an existing
   database/WAL pair. Never run the old and restored instance concurrently
   against the same database.
3. Start the same or a newer compatible server version. Check `/health`, log
   into the admin panel, inspect **Activity**, and verify an enrolled device can
   refresh. Compare a known receipt and its signing public key with your
   retained export. Keep the prior data until that check succeeds.

The server regression suite takes a consistent SQLite `VACUUM INTO` snapshot,
copies the audit files, opens the result through normal startup, and verifies
that a receipt still verifies after its device was revoked and its user was
deleted. This tests the data format and restore path; operators should also
practice restoring their actual backup mechanism. Clients' private signing
keys live on the clients, so a server backup does not replace a device backup.

## Callback, audit, and delivery policies

These settings and behaviors describe the current source build. Upgrade the
server and standalone APNs relay together for the added device routing metadata
and structured delivery errors.

Callbacks run after a decision is committed and do not delay its response.
Each has a ten-second timeout; redirects are disabled and a failure's response
body is capped at 4 KiB. At most sixteen callbacks run concurrently. A full
callback pool records a failure in the audit log. Callbacks are best effort,
with no durable retry queue; issuers needing reliable completion should use the
request decision/read or wait endpoints to reconcile outcomes after a timeout
or server restart.

Restrict callbacks to destinations you control with a top-level TOML setting:

```toml
callback_allowed_origins = ["https://automation.example.com", "http://127.0.0.1:8080"]
```

Or set `NOD_CALLBACK_ALLOWED_ORIGINS` to a comma-separated list. Matching uses
the exact scheme, host, and effective port; entries cannot include credentials,
paths other than `/`, query strings, or fragments. Callback URLs may have paths
and queries but cannot contain userinfo. An empty list (or an empty environment
value) disables callbacks. Omitting this setting preserves the existing
trusted-issuer policy, allowing any HTTP(S) destination reachable by Nod.
The allowlist does not isolate network egress or pin DNS resolution; use an
egress firewall when that boundary is required.

Push jobs are stored atomically with request creation and recovered after a
restart. The server sends at most four concurrently and makes at most three
attempts per device/request, with bounded timeouts and short backoff for
transient failures. Permanent APNs rejections do not retry; an invalid-token
rejection clears that token only if it is still current. A matching registered
iOS/watchOS device reports push delivery; devices without a usable route or
token report WebSocket delivery. Push acceptance does not prove the user saw a
notification. The admin Activity page distinguishes queued, sending, accepted,
failed, and skipped jobs. After a crash, a send may be retried even if Apple
already accepted it; a stable APNs collapse ID reduces duplicate alerts.

The audit log rotates at 16 MiB into uniquely named archives. Rotation keeps
all archives; operators own archive retention, protected backup, and disk-space
monitoring. A failed write or rotation makes admin Activity report unhealthy
until restart, because later writes cannot repair a lost entry. Audit writes
are flushed but are not a transaction with SQLite and do not claim power-loss
durability. Decision receipts and device verification keys also remain in
SQLite after device revocation or user deletion.

`NOD_RETENTION_DAYS` applies to terminal requests since their resolution or last
status change. Pending requests survive that window; set an explicit expiry
when unattended requests should time out. Clearing a channel hides its handled
items at that point in time and preserves pending work. History uses
`include_cleared=true`; request lists support `channel_id`, `search`, `limit`,
and the returned opaque `next_cursor` as `before`. The first page includes all
matching pending requests plus up to 100 handled requests by default; `limit`
is clamped to 0–500. Later pages contain handled requests only. Search covers
title, summary, and body. Explicit recipients override channel subscriptions;
requests without explicit recipients target the channel's subscribers.

For retries after an uncertain create response, send an `idempotency_key` with
the HTTP create request and reuse it with the same request content. It is scoped
to the issuer and channel and returns the original request even after it has
been handled; changing the content under that key returns a conflict. Keys last
as long as their request remains stored. The separate `dedupe_key` continues to
coalesce only pending requests.


Per-device notification preferences synchronize through
`PUT /api/v1/devices/me/notification-preferences` and appear on
`UserDevice.notification_preferences`: `hide_content`, `muted_channels`, and
`snoozed_until`. They preserve inbox content while controlling alerts. Server
push workers recheck mute/snooze immediately before delivery, and personal
`hide_content` forces generic previews even when an issuer supplied preview
text. Suppressed pushes are marked skipped and are not replayed when snooze
ends. Use operating-system Focus/Do Not Disturb for recurring quiet hours.
