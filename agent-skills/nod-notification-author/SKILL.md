---
name: nod-notification-author
description: Design high-quality Nod request payloads for automations, agents, scripts, and services, including audience targeting, decision behavior, options, callback wake-ups, dedupe, expiry, and privacy-safe notification text.
---

# Nod Notification Author

Use this skill when an agent needs to create, review, or improve a Nod request
payload for a human-facing automation.

## Workflow

1. Identify the human decision needed, the issuer service, and the risk of
   getting no answer or the wrong answer.
2. Choose the audience:
   - Whole channel: set `channel_id` and omit `recipients`.
   - Specific users: set `recipients` to user ids on that channel.
3. Choose the decision mode:
   - `shared`: first valid response resolves the request for everyone.
   - `per_user`: each recipient records a decision; the request resolves after
     all recipients respond.
   - Quorum: Nod v1 has no server-side quorum field. Use `per_user`, then have
     the issuer read or wait for decisions and decide when enough responses have
     arrived.
4. Write copy for a small card, not a log dump:
   - `title`: the decision in one sentence.
   - `summary`: the shortest useful scan line.
   - `body_markdown`: the why, consequence, deadline, and next step.
   - `fields`: short facts that should be easy to compare.
   - `links`: runbooks, diffs, dashboards, receipts, or source records.
5. Choose options that map to real outcomes. Use `approve_with_text` or
   `reject_with_text` when a comment matters.
6. Add delivery behavior:
   - `dedupe_key` for retry-safe automations.
   - `expires_at` for stale decisions.
   - issuer-side wait/read when the automation needs the result.
   - `callback_url` only as an unauthenticated wake-up hint; the issuer must
     read or wait before acting.
7. Protect privacy. Put sensitive details in the request body, fields, or links;
   use `notification.redact` for push-safe lock-screen text.
8. Validate against the request contract before sending.

## References

- Read `references/request-contract.md` before generating new payload shapes or
  reviewing field names.
- Read `references/payload-examples.md` for copyable JSON patterns.
- Read `references/validation-and-privacy.md` before testing a sender, handling
  secrets, callback wake-ups, APNs, or local client smoke checks.

## Output Standard

When authoring a request, produce:

- The final JSON payload.
- A short note naming the audience and decision mode.
- Any issuer-side follow-up needed, especially for quorum.
- A privacy note if push notifications may be shown on a lock screen.
