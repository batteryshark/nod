<p align="center">
  <img src="../../assets/nod-icon.png" width="180" alt="Nod">
</p>

# nod-server

Self-hosted decision protocol server for personal agents, automations, and services. Accepts decision requests, stores signed decision records, pushes requests to registered devices, streams state changes over WebSockets, and writes append-only JSONL audit logs.

This service lives under `server/nod-server` in the Nod monorepo. The native Apple clients (macOS, iOS) live at [client/nod-apple](../../client/nod-apple).

## What's in the box

- `axum` + `tokio` + `sqlx` on SQLite (WAL), with append-only JSONL audit logs
- Admin-created users/channels, short-lived enrollment codes, device tokens, and issuer tokens
- Request payloads with rendered message snapshots, fields, links, optional image URL, APNs notification redaction metadata, dedupe key, and structured options
- Agent-friendly decision API, wait API, and optional callback wake-up URL
- Signed device decisions using P-256 ECDSA/SHA-256 keys registered during enrollment
- User-targeted delivery with shared or per-user decision resolution
- WebSocket sync for `created` / `resolved` / `expired` / `cancelled` / `cleared` / subscription / channel-update events
- Notification delivery over either remote push or WebSocket/local notifications, with the APNs relay as an optional remote-push route
- Server-hosted admin panel at `/admin` for users, channels, devices, enrollment codes, issuer tokens, and health
- Docker Compose for one-command deploys

Designed to run behind a private tunnel (Tailscale Serve, etc.) — never expose it directly to the public internet.

Issuer read and wait endpoints are authoritative for automation results. A
request `callback_url` receives an unsigned, best-effort POST after a recorded
decision and should only be used as a wake-up hint; callback receivers must not
act on the webhook body without reading or waiting for the decision through
Nod's authenticated API.

Deploying on a laptop or a small box? Start with the prebuilt-binary guide in
[docs/deploy.md](../../docs/deploy.md) — this README is the deep-dive on
configuration, Docker, and APNs.

## Run locally

```bash
export NOD_ADMIN_TOKEN="replace-this"
cargo run -p nod-server
```

```bash
curl -s http://127.0.0.1:8767/health
```

Admin panel: `http://127.0.0.1:8767/admin`. Logging in with `NOD_ADMIN_TOKEN` sets a 12-hour signed HttpOnly cookie. Bearer admin-token auth still works for scripts.

The admin page is embedded in the binary. When iterating on it, point
`NOD_ADMIN_HTML_PATH` at the source file and edits show on refresh without a
rebuild (the dev Compose file does this for you). From the repo root:

```bash
NOD_ADMIN_HTML_PATH=server/nod-server/assets/admin.html cargo run -p nod-server
```

## Configuration

The server reads non-secret settings from built-in defaults, then an optional
`NOD_CONFIG` TOML file, then environment overrides. Keep the TOML safe to
commit: `admin_token`, APNs keys, and relay client certificates should be
injected as environment values or mounted files.

Use `config.example.toml` for non-secret options and `secrets.example.env` as the
shape for secret injection. Injected values also support common file mounts:
`NOD_ADMIN_TOKEN_FILE`, `NOD_APNS_RELAY_CLIENT_CERT_PATH_FILE`,
`NOD_APNS_RELAY_CLIENT_KEY_PATH_FILE`,
and `NOD_APNS_RELAY_CA_CERT_PATH_FILE`.

## Docker

Build and run the production image directly when you want a small personal
instance without Compose. From `server/nod-server/`:

```bash
docker build -t nod-server:local .
docker volume create nod-data
export NOD_ADMIN_TOKEN="$(openssl rand -base64 48)"
printf 'NOD_ADMIN_TOKEN=%s\n' "$NOD_ADMIN_TOKEN"
docker run -d --name nod --restart unless-stopped \
  -p 127.0.0.1:8767:8767 \
  -e NOD_ADMIN_TOKEN="$NOD_ADMIN_TOKEN" \
  -v nod-data:/data \
  nod-server:local
```

The image stores SQLite data and audit logs under `/data`, runs as an
unprivileged user, and includes a `/health` Docker healthcheck.

## Docker Compose

Keep runtime secrets in `secrets/secrets.env`. This file is ignored by git, and
`scripts/nod-compose` will create it with a generated admin token if one is
missing:

```bash
NOD_ADMIN_TOKEN=replace-this
```

Then start the service. It binds to `127.0.0.1:8767` for Tailscale Serve:

```bash
scripts/nod-compose up -d --build
```

### Local APNs relay

For a same-machine deployment with production APNs push, run the standalone
relay as a private Compose sidecar. The relay has no host-published port; the
server reaches it only on the Compose network at
`https://nod-apns-relay:8768`.

Initialize the relay env file and local mTLS material:

```bash
scripts/nod-compose --with-apns-relay config >/dev/null
```

This creates `secrets/relay.env` if needed and generates local mTLS files under
`secrets/`. The local relay CA private key is kept in `.relay-ca/`, outside the
container-mounted secrets directory.

Copy your Apple APNs `.p8` key into `secrets/` and fill these values in
`secrets/relay.env`:

```bash
NOD_APNS_RELAY_TEAM_ID=...
NOD_APNS_RELAY_KEY_ID=...
NOD_APNS_RELAY_PRIVATE_KEY_PATH=/secrets/AuthKey_....p8
NOD_APNS_RELAY_BUNDLE_ID=com.batteryshark.Boop
NOD_APNS_RELAY_ENVIRONMENT=production
```

Then start Nod with the relay enabled:

```bash
scripts/nod-compose --with-apns-relay up -d --build
```

To rotate the local relay mTLS material:

```bash
scripts/nod-relay-init --force
scripts/nod-compose --with-apns-relay up -d --force-recreate
```

Verify mTLS from the Nod container:

```bash
scripts/nod-compose --with-apns-relay exec nod \
  curl -fsS \
    --cert /secrets/relay-client.crt \
    --key /secrets/relay-client.key \
    --cacert /secrets/relay-ca.crt \
    https://nod-apns-relay:8768/health
```

For active development, use the dev image. It bind-mounts the repo and keeps Cargo caches in Docker volumes, so restarting recompiles only changed Rust code:

```bash
scripts/nod-dev up -d --build
scripts/nod-dev restart nod
scripts/nod-dev logs -f nod
```

## Tailscale HTTPS

```bash
tailscale serve --bg --set-path /nod 8767
```

Then point clients and issuers at `https://<your-tailnet-host>/nod`.

## Push Providers

The core server uses generic push-provider device fields. Apple devices register
with `push_provider = "apple_apns"`, `native_app_id` set to the bundle id/APNs
topic, and a provider token. Push registrations without a native app id are
rejected.

Device-facing APIs report notification delivery as either `push` or `websocket`.
The `push` mode means the server has a configured APNs route. The
`websocket` mode means Apple clients should present `created` sync events as
local notifications while connected. On iOS, WebSocket/local delivery is
foreground-only; background and lock-screen delivery still require APNs.

There are two mutually exclusive APNs routes, selected by which config block
is present:

- **In-process direct** (`notifications.apns_direct` / `NOD_APNS_DIRECT_*`):
  the server talks to Apple itself using a `.p8` key on the same box —
  `NOD_APNS_DIRECT_BUNDLE_ID`, `NOD_APNS_DIRECT_TEAM_ID`,
  `NOD_APNS_DIRECT_KEY_ID`, `NOD_APNS_DIRECT_PRIVATE_KEY_PATH`. Right fit for
  a single-operator, single-machine deployment.
- **Remote relay** (`notifications.apns_relay` / `NOD_APNS_RELAY_*`): pushes
  go through the standalone mTLS relay below, keeping Apple credentials out
  of the main server. Configuring both routes is an error.

## APNs Relay

For credential isolation, a Nod server can send remote notifications through
the standalone APNs relay instead of the in-process route. The server and
relay communicate over mTLS; bearer tokens are not used on this hop:

```bash
NOD_APNS_RELAY_URL=https://relay.example.com:8768
NOD_APNS_RELAY_NATIVE_APP_ID=com.yourname.Boop
NOD_APNS_RELAY_CLIENT_CERT_PATH=/secrets/relay-client.crt
NOD_APNS_RELAY_CLIENT_KEY_PATH=/secrets/relay-client.key
NOD_APNS_RELAY_CA_CERT_PATH=/secrets/relay-ca.crt
```

Request creation still succeeds if relay delivery fails; the server logs the
push failure and keeps the request available for sync.
The relay route is operator-facing only; device clients still see
`notification_delivery.mode = "push"`.

Host the relay with the sibling [server/nod-apns-relay](../nod-apns-relay) project:

```bash
NOD_APNS_RELAY_SERVER_CERT_PATH=/secrets/relay-server.crt
NOD_APNS_RELAY_SERVER_KEY_PATH=/secrets/relay-server.key
NOD_APNS_RELAY_CLIENT_CA_CERT_PATH=/secrets/relay-client-ca.crt
NOD_APNS_RELAY_TEAM_ID=...
NOD_APNS_RELAY_KEY_ID=...
NOD_APNS_RELAY_BUNDLE_ID=com.yourname.Boop
NOD_APNS_RELAY_PRIVATE_KEY_PATH=/secrets/AuthKey_....p8
NOD_APNS_RELAY_ENVIRONMENT=production
cargo run
```

The APNs relay serves only `/health` and `POST /v1/notifications`;
it does not open the Nod database or write audit logs.

## Issuer example

A minimal Python sender lives at [examples/issuer.py](examples/issuer.py).

## Docs

- [API reference](docs/API.md)

## Verify

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`scripts/nod-smoke` runs the end-to-end check (enroll → request → WebSocket
sync → signed decision). With no arguments it spins up an in-process server;
point it at a running instance to verify a deployment:

```bash
scripts/nod-smoke
scripts/nod-smoke https://nod.example "$NOD_ADMIN_TOKEN"
```

The deployed form creates uniquely-suffixed `smoke-` resources and removes
them on success (the issuer token is revoked rather than deleted, so one
revoked `smoke-` row remains per run).

## License

[AGPL-3.0](../../LICENSE)
