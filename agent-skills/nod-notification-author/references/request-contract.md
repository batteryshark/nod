# Nod Request Contract

Nod request creation is strict. `POST /api/v1/requests` rejects unknown top-level
fields, so use the names below exactly.

## Create Request Fields

Required:

- `title`: non-empty, trimmed, maximum 200 characters.

Common optional fields:

- `channel_id`: ASCII slug, defaults to `default`.
- `recipients`: non-empty list of user ids when targeting specific users.
- `decision_resolution`: `shared` or `per_user`, defaults to `shared`.
- `summary`: short scan line. If empty, Nod derives it from the first body line.
- `body_markdown`: detailed body shown inside Nod clients.
- `fields`: list of `{ "label": "...", "value": "...", "style": "..." }`.
- `links`: list of `{ "label": "...", "url": "https://..." }`.
- `image_url`: optional image URL for clients that display one.
- `notification`: APNs alert presentation controls.
- `dedupe_key`: retry id for a pending request in the same channel.
- `expires_at`: RFC 3339 UTC timestamp.
- `options`: list of request options.
- `callback_url`: absolute `http` or `https` URL called after a decision as an
  unsigned wake-up hint only.
- `template_id`, `template_version`, `variables`: accepted metadata for rendered
  templates. Nod stores the rendered request snapshot, not the template inputs.

Do not use `source_id`; use `channel_id`.

## Audience

Whole-channel fanout:

- Set `channel_id`.
- Omit `recipients`.
- Nod sends to every subscribed user on that channel.

Specific-user targeting:

- Set `channel_id`.
- Set `recipients` to user ids.
- Only those users can see or answer the request.
- Duplicate recipients are collapsed.
- An empty `recipients` array is invalid.

## Decision Resolution

`shared`:

- Default behavior.
- First valid device response resolves the request.
- All recipients see the same final decision.
- Good for "any authorized person can approve" workflows.

`per_user`:

- Every targeted recipient has an independent decision.
- The aggregate request stays pending until all recipients decide.
- The decision read API includes `decisions` and `pending_recipients`.
- Good for acknowledgments, roll calls, and multi-person confirmations.

Quorum:

- Nod v1 has no `quorum`, `required_count`, voting, or threshold field.
- Implement quorum in the issuer:
  1. Send a `per_user` request.
  2. Poll or wait on `/api/v1/requests/{request_id}/decision`.
  3. Count acceptable entries in `decisions`.
  4. Act when the external quorum rule is satisfied.
  5. Optionally ignore late responses or cancel pending work in the issuer.

## Options

Option fields:

- `id`: ASCII slug, unique within the request.
- `label`: human-facing button label, required.
- `kind`: one of `approve`, `approve_with_text`, `reject`,
  `reject_with_text`, `dismiss`, `open`, `custom`.
- `style`: optional client hint, defaults to `default`.
- `requires_text`: optional boolean. Nod forces this to `true` for
  `approve_with_text` and `reject_with_text`.
- `text_placeholder`: optional prompt shown by clients for text options.
- `destructive`: optional client hint for dangerous actions.
- `foreground`: optional client hint for options that should open the app.

If a request has no options, clients can still submit the implicit `dismiss`
option. Use optionless requests only for informational cards.

## Notification

`notification` only controls APNs alert text:

```json
{
  "redact": true,
  "title": "Nod",
  "body": "Open Nod to review this request."
}
```

If `redact` is false or omitted, APNs uses the request title and summary unless
custom notification title/body are provided. If `redact` is true and title/body
are omitted, Nod uses generic safe defaults.

Requests do not set notification sound. Sound comes from device/server
notification preferences. APNs delivery is best effort; WebSocket sync and
decision reads are the source of truth.

## Callback Behavior

When `callback_url` is set, Nod sends an unsigned, best-effort POST after a
decision is recorded. Treat it as a wake-up hint only: authenticate the receiver
route yourself, ignore forged or duplicate deliveries, and read or wait for the
decision through Nod's authenticated API before acting.

The payload contains:

```json
{
  "request_id": "req_123",
  "channel_id": "deploys",
  "status": "resolved",
  "decision": {
    "request_id": "req_123",
    "option_id": "approve",
    "option_kind": "approve",
    "option_label": "Approve",
    "text": "ship it",
    "actor_user_id": "owner",
    "actor_device_id": "device_123",
    "signature": null,
    "resolved_at": "2026-06-10T16:30:00.000Z"
  },
  "decisions": [],
  "decision_resolution": "shared"
}
```

Callback failures are logged and audited, but they do not undo the recorded
decision. Issuers that need a durable or security-sensitive result must read or
wait for the decision.
