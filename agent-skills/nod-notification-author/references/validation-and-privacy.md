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

The create response returns `request_id`, `deduped`, and `request`. If
`deduped` is true, Nod returned an existing pending request for the same
`channel_id` and `dedupe_key`.

## Read Or Wait For Decisions

```bash
curl -sS "$NOD_BASE_URL/api/v1/requests/$REQUEST_ID/decision" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN"
```

```bash
curl -sS "$NOD_BASE_URL/api/v1/requests/$REQUEST_ID/wait?timeout_seconds=55" \
  -H "Authorization: Bearer $NOD_ISSUER_TOKEN"
```

For `per_user` requests, inspect `decisions` and `pending_recipients`. For
external quorum, the issuer is responsible for counting decisions and deciding
when to proceed.

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
  source of truth. Read or wait for the decision before acting.
- Keep callback URLs authenticated, stable, and idempotent. Nod may log
  callback failures but will not undo the decision.
