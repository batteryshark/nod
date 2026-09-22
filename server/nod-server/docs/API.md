# Nod API

Protected JSON endpoints use `Authorization: Bearer <token>`. Admin endpoints
also accept the signed `nod_admin_session` cookie set by `/admin/session`.

Token types:

- Admin token: configured by `NOD_ADMIN_TOKEN`.
- Issuer token: created by the admin API; used by agents/services to create
  requests, read decisions, and optionally cancel their own pending requests.
- Device token: returned by `/api/v1/enroll`; used by native clients for
  transport authentication.

## Bootstrap

```bash
export NOD_ADMIN_TOKEN="replace-this"
cargo run -p nod-server
```

Create an issuer token:

```bash
curl -sS -X POST http://127.0.0.1:8767/api/v1/admin/issuer-tokens \
  -H "Authorization: Bearer $NOD_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"agents","scopes":["requests:write","requests:read"]}'
```

Create an enrollment code:

```bash
curl -sS -X POST http://127.0.0.1:8767/api/v1/admin/users/owner/enrollment-codes \
  -H "Authorization: Bearer $NOD_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"expires_in_seconds":600}'
```

Enroll a device with a decision signing key:

```json
{
  "code": "ABCDEFGH",
  "device_name": "iPhone",
  "platform": "ios",
  "native_app_id": "com.batteryshark.Boop",
  "push_provider": "apple_apns",
  "push_token": "provider-token",
  "signing_key": {
    "key_id": "device-key-id",
    "algorithm": "p256_ecdsa_sha256",
    "public_key": "base64url-x963-p256-public-key"
  },
  "attestation": {
    "provider": "apple_app_attest",
    "key_id": "app-attest-key-id",
    "attestation_object": "base64url-cbor-attestation-object"
  }
}
```

`attestation` is optional. When Apple App Attest is configured, the server
verifies it in report-only mode during enrollment and stores only a sanitized
summary: provider, status, key id, app identity, verification time, future
assertion key material, counter, receipt hash, and failure reason. Raw
attestation objects are never stored. Unsupported platforms and clients that
omit attestation can still enroll.

Configure Apple App Attest explicitly:

```toml
[device_attestation.apple_app_attest]
mode = "report_only"
team_id = "Y734633UDM"
bundle_ids = ["com.batteryshark.Boop"]
environment = "production"
```

TestFlight, App Store, and Apple Developer Enterprise Program distributions
operate in the production App Attest environment, so paired release builds should
be verified against `production`.

`native_app_id` is required whenever `push_provider` and `push_token` are
present. For Apple APNs it must be the bundle id/APNs topic, such as
`com.batteryshark.Boop`.

Device-facing responses from `/api/v1/enroll`, `/api/v1/users/me`, and the
WebSocket `hello` envelope include required notification delivery metadata:

```json
{
  "notification_delivery": {
    "mode": "push"
  }
}
```

`mode` is either `push` or `websocket`. `push` means this iOS/watchOS device
has a usable token and a matching configured APNs provider/bundle route. `websocket` means clients should present `created` WebSocket sync
events as local notifications while connected. APNs routing is transparent to
device clients.

## Requests

Create a decision request:

```bash
curl -sS -X POST http://127.0.0.1:8767/api/v1/requests \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel_id":"default",
    "title":"Approve deploy",
    "summary":"Production deploy is waiting",
    "body_markdown":"**Production** deploy needs approval.",
    "notification":{
      "redact":true,
      "title":"Nod",
      "body":"Open Nod to review this request."
    },
    "dedupe_key":"deploy:prod:42",
    "options":[
      {"id":"approve","label":"Approve","kind":"approve"},
      {"id":"approve_notes","label":"Approve with notes","kind":"approve_with_text","text_placeholder":"Notes"},
      {"id":"reject","label":"Reject","kind":"reject"}
    ]
  }'
```

`notification` is optional and controls APNs and current clients' local alert
presentation. Without it, previews use the title and summary, with a safe body
fallback for title-only requests. With `"redact": true`, previews use the
provided notification title/body, or generic defaults when omitted. Images and
request details are suppressed in redacted native alerts; full content remains
in the authenticated inbox. Personal device `hide_content` overrides even the
issuer's supplied preview text.

Issuer/admin full responses retain `request.request_digest` for the original
`nod-request-v1` canonicalization. Current clients use the optional
`request.signing` context instead:

```json
{
  "version": "nod-request-v2",
  "recipients_commitment": "64-character-sha256-hex",
  "request_digest": "64-character-sha256-hex"
}
```

Clients independently recompute the v2 digest over the visible immutable
content and salted recipient commitment before signing; they must reject an
unknown context version or a mismatch. The server's secret per-request salt
prevents guessing hidden recipient IDs from the commitment. Multi-recipient
device projections omit the unsalted v1 `request_digest`, which otherwise
allows such guessing. Single-recipient views retain it for installed v1 clients.
Historical v1 receipts remain verifiable and may still contain their original
unsalted digest. The canonical implementations and frozen vectors live in
[`nod-proto/src/signing.rs`](../../../nod-proto/src/signing.rs).

`recipients` explicitly targets users even when they have unsubscribed from the
channel. When omitted, Nod targets the current subscribers. `decision_resolution`
can be `shared` (one decision resolves the request) or `per_user` (each recipient
resolves their own copy). Device views hide other recipients and per-user
receipts.

Use an optional `idempotency_key` for retries after an uncertain HTTP response.
It is 1–128 bytes, scoped to the issuer and channel, and returns the same request
even after resolution or cancellation. Reusing it with changed parsed content
returns 409. Its record survives until the request is removed by retention or
channel deletion. `dedupe_key` retains its separate pending-only behavior.
Both fields are accepted by `/api/v1/admin/test-requests`.

Requests may include `callback_url`, but callback delivery is an unsigned,
best-effort wake-up after Nod records a decision. Do not treat the callback
payload as proof of approval; use the authenticated read or wait endpoint before
acting.

List visible requests for a registered device:

```bash
curl -sS "http://127.0.0.1:8767/api/v1/requests?limit=500" \
  -H "Authorization: Bearer $NOD_DEVICE_TOKEN"
```

The response is `{ "requests": [...], "next_cursor": "..." }`, with a null
cursor at the end. Optional query fields are `channel_id`, `search`,
`include_cleared`, `limit` (default 100, clamped to 0–500), and `before` (the
previous `next_cursor`). The first page contains all matching pending requests
plus one handled page; subsequent pages contain only handled requests, ordered
by creation time and ID descending. Search matches title, summary, and body.
Use a fresh first page after changing the search/channel filter. Invalid or
oversized cursors return 400; cursor contents are not authorization.

`POST /api/v1/devices/me/channels/{channel_id}/clear` hides handled items for
that user while keeping pending requests visible. `include_cleared=true` makes
hidden history browsable. Requests handled after a clear remain visible.
Retention removes terminal requests after their resolution/status-change window;
pending requests remain until handled or explicitly expired.

Read or wait for decisions:

```bash
curl -sS http://127.0.0.1:8767/api/v1/requests/$REQUEST_ID/decision \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN"

curl -sS "http://127.0.0.1:8767/api/v1/requests/$REQUEST_ID/wait?timeout_seconds=55" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN"
```

Submit a signed decision:

```json
{
  "text": "ship it",
  "signature": {
    "key_id": "device-key-id",
    "algorithm": "p256_ecdsa_sha256",
    "nonce": "unique-device-nonce",
    "signed_at": "2026-05-31T12:00:00.000Z",
    "request_digest": "server-provided-request-digest",
    "signature": "base64url-der-ecdsa-signature"
  }
}
```

The signed payload is the UTF-8 string:

```text
nod-decision-v1
request_id:<request id>
request_digest:<request digest>
option_id:<option id>
option_kind:<option kind>
user_id:<user id>
device_id:<device id>
key_id:<key id>
nonce:<nonce>
signed_at:<UTC timestamp with milliseconds>
text_sha256:<sha256 hex of trimmed response text>
```

The server verifies the P-256 ECDSA/SHA-256 signature, rejects nonce reuse per
device key, and stores the signature metadata on the decision record.


Text-capable options (`requires_text` and `*_with_text` kinds) display a text
input; text remains optional in the existing protocol. Clients should label it
optional. An implicit `dismiss` is valid only when the request has no options.

## Per-device alert controls

`PUT /api/v1/devices/me/notification-preferences` replaces the current device's
preferences and returns `{ "ok": true }`:

```json
{
  "hide_content": true,
  "muted_channels": ["builds"],
  "snoozed_until": "2026-09-23T12:00:00Z"
}
```

Defaults are `false`, `[]`, and `null`. The settings appear on
`UserDevice.notification_preferences`, including `/api/v1/users/me`'s
`current_device`. Channel IDs must be valid slugs, at most 100 entries; duplicates
are normalized. An active snooze or a muted channel suppresses this device's
alerts while requests remain in its inbox. Hidden content replaces all native
preview text with safe defaults. Preference updates emit `device_preferences_updated`
for clients to refresh; push token changes emit `device_push_updated`. Both are
scoped to the owning user. Use OS Focus/Do Not Disturb for recurring quiet hours.

## Operations and receipt evidence

`GET /api/v1/admin/activity` returns the latest 50 full requests, latest 100 push
delivery records, and `{ "healthy": bool, "last_error": string|null }` audit
health. Each delivery includes `request_id`, `device_id`, `status`, `attempts`,
`updated_at`, and `error`; statuses are `queued`, `sending`, `accepted`, `failed`,
and `skipped`. Accepted means the upstream accepted the push, not that a person
saw it. See [delivery policies and backup/restore](../../../docs/deploy.md#callback-audit-and-delivery-policies).

New signature receipts include `public_key` alongside the canonical
`signing_payload` and signature. Device revocation and user deletion preserve
historical device keys, recipients, and receipts; revoked credentials cannot
open or retain an authorized WebSocket. Deleted user IDs remain reserved.
Installed legacy records may omit `public_key`; retained device records are the
historical verifier. Audit archives are independently retained when terminal
requests leave SQLite retention.
