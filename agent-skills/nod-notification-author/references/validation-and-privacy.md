# Validation And Privacy

## Submit With An Issuer Token

Use issuer tokens for automations. The token needs write scope for the target
channel, such as `requests:write` or `requests:write:deploys`.

```bash
curl -sS -X POST "$NOD_BASE_URL/api/v1/requests" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN" \
  -H "Content-Type: application/json" \
  -d @payload.json
```

The create response returns `request_id`, `deduped`, and `request`. Save the
whole body (`created.json`): it is the input `nod-verify` recomputes the digest
from. If `deduped` is true, Nod returned an existing pending request for the
same `channel_id` and `dedupe_key`; compare its `request` content with what
you sent before treating its decision as yours.

## Read Or Wait For Decisions

```bash
curl -sS "$NOD_BASE_URL/api/v1/requests/$REQUEST_ID/decision" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN" > decision.json
```

```bash
curl -sS "$NOD_BASE_URL/api/v1/requests/$REQUEST_ID/wait?timeout_seconds=55" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN" > decision.json
```

For `per_user` requests, inspect `decisions` and `pending_recipients`. For
external quorum, the issuer is responsible for counting decisions and deciding
when to proceed — and for verifying each counted decision.

## Verify Before Acting

Reading or waiting tells you what the server recorded. It does not tell you a
principal's device signed it: the server asserts `verified: true` and attaches a
`public_key`, and a compromised server can assert anything. The final authority
is `nod-verify`, run with keys you pinned out of band:

```bash
nod-verify --request created.json --decision decision.json --keys keys.json \
  --expect-user owner --expect-option approve
```

- Exit `0`: a pinned device signed exactly this option on exactly the request
  content in `created.json`. Act.
- Exit `1`: the receipt does not verify (null signature, unknown key, digest or
  payload mismatch, wrong option, expectation failed, or no decision yet). Do
  not act.
- Exit `2`: an input could not be read or understood. Fix the input; do not act.

Stdout is one JSON line naming the verified `user_id`, `device_id`, `key_id`,
`option_id` and `option_kind`, plus any warnings (for example `signed_at` far
from your clock; add `--strict-time` to make that a failure).

`keys.json` holds the principal's device keys, copied from the admin page's
Devices table or `GET /api/v1/admin/devices` (`signing_key_id`,
`signing_public_key`) when the device was enrolled, and stored where the Nod
server cannot write:

```json
{
  "keys": [
    { "user_id": "owner", "device_id": "device_123", "key_id": "device-key-id", "public_key": "BC…" }
  ]
}
```

Never populate `keys.json` from a decision response. For `per_user` requests
run `nod-verify --expect-user <user>` once per decision you intend to count.
`nod-verify` is in every release next to `nod-server`; Rust issuers can call
`nod_proto::verify_decision_receipt` directly.

## Smoke Test With Clients

Use at least one enrolled client before trusting a new automation:

- TUI: enroll it, keep it connected, and confirm the request appears live.
- Desktop: confirm the card appears and action buttons behave as expected.
- iOS: confirm foreground sync works. Background or lock-screen delivery needs
  APNs configured.

For deployment-level verification, run:

```bash
server/nod-server/scripts/nod-smoke "$NOD_BASE_URL" "$NOD_ADMIN_TOKEN"
```

## Privacy Rules

- Never put secrets, tokens, one-time codes, private keys, customer PII, or
  medical/financial details into `title`, `summary`, or unredacted
  `notification` text.
- Assume push alert title/body may appear on a lock screen, notification center,
  device logs, or a paired wearable.
- Prefer generic push text with `notification.redact: true` for sensitive
  requests.
- Put sensitive context behind authenticated links or inside the request body
  only when the enrolled audience is allowed to see it.
- Use `fields` for short facts, not raw logs.
- Use `links` for large artifacts, traces, diffs, dashboards, and runbooks.

## Expiry, Dedupe, And Callbacks

- Add `dedupe_key` for idempotent automation retries.
- Add `expires_at` when a late answer would be harmful or meaningless.
- Treat callbacks as unsigned wake-up hints, not as approval evidence or the
  source of truth. Read or wait for the decision, then run it through
  `nod-verify` with your pinned keys; act only on exit 0.
- Keep callback URLs authenticated, stable, and idempotent. Nod may log
  callback failures but will not undo the decision.
