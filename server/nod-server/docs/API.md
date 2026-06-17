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

`mode` is either `push` or `websocket`. `push` means the server has an effective
APNs route. `websocket` means clients should present `created` WebSocket sync
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

`notification` is optional and only controls APNs alert presentation. Without it,
APNs uses the request title and summary. With `"redact": true`, APNs uses the
provided notification title/body, or safe generic defaults when they are omitted;
the request body, fields, links, and options remain available only after the
client opens/fetches the request.

The response includes `request.request_digest`, which clients sign when recording
a decision.

Requests may include `callback_url`, but callback delivery is an unsigned,
best-effort wake-up after Nod records a decision. Do not treat the callback
payload as proof of approval; use the authenticated read or wait endpoint before
acting.

List visible requests for a registered device:

```bash
curl -sS "http://127.0.0.1:8767/api/v1/requests?limit=500" \
  -H "Authorization: Bearer $NOD_DEVICE_TOKEN"
```

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
