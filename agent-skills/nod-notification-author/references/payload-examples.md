# Nod Payload Examples

These examples are request bodies for `POST /api/v1/requests`.

## Approve Or Reject

```json
{
  "channel_id": "deploys",
  "title": "Approve production deploy?",
  "summary": "Release v1.4.2 is ready for prod",
  "body_markdown": "CI passed and the canary is healthy. Approving starts the production deploy.",
  "fields": [
    { "label": "Service", "value": "api" },
    { "label": "Version", "value": "v1.4.2" },
    { "label": "Risk", "value": "Low" }
  ],
  "links": [
    { "label": "Diff", "url": "https://example.com/compare/v1.4.1...v1.4.2" },
    { "label": "Runbook", "url": "https://example.com/runbooks/deploy" }
  ],
  "dedupe_key": "deploy:api:v1.4.2",
  "options": [
    { "id": "approve", "label": "Approve", "kind": "approve" },
    { "id": "reject", "label": "Reject", "kind": "reject", "destructive": true }
  ]
}
```

## Approve With Comment

```json
{
  "channel_id": "deploys",
  "title": "Approve migration with notes?",
  "summary": "Billing migration needs an explicit operator note",
  "body_markdown": "The migration has been tested against staging. Add any rollout note the automation should attach to the deploy record.",
  "fields": [
    { "label": "Migration", "value": "20260610_add_invoice_events" },
    { "label": "Estimated duration", "value": "90 seconds" }
  ],
  "options": [
    {
      "id": "approve_notes",
      "label": "Approve with note",
      "kind": "approve_with_text",
      "text_placeholder": "Rollout note"
    },
    {
      "id": "reject_reason",
      "label": "Reject with reason",
      "kind": "reject_with_text",
      "text_placeholder": "What should change?"
    }
  ]
}
```

## Informational Or Dismiss Only

```json
{
  "channel_id": "builds",
  "title": "Nightly build finished",
  "summary": "All platform artifacts are ready",
  "body_markdown": "The nightly build completed and checksums were uploaded. No decision is required.",
  "fields": [
    { "label": "Build", "value": "2026-06-10.42" },
    { "label": "Artifacts", "value": "server, tui, desktop" }
  ],
  "links": [
    { "label": "Artifacts", "url": "https://example.com/builds/2026-06-10.42" }
  ],
  "dedupe_key": "nightly:2026-06-10"
}
```

## Per-User Acknowledgment

```json
{
  "channel_id": "ops",
  "recipients": ["alex", "maya", "owner"],
  "decision_resolution": "per_user",
  "title": "Acknowledge incident handoff",
  "summary": "Each on-call person must acknowledge the handoff",
  "body_markdown": "Please confirm that you have read the handoff and know who owns the next action.",
  "fields": [
    { "label": "Incident", "value": "INC-2026-0610" },
    { "label": "Current state", "value": "Monitoring after rollback" }
  ],
  "options": [
    { "id": "ack", "label": "Acknowledged", "kind": "approve" },
    {
      "id": "ack_with_note",
      "label": "Ack with note",
      "kind": "approve_with_text",
      "text_placeholder": "Optional handoff note"
    }
  ],
  "expires_at": "2026-06-10T22:00:00.000Z"
}
```

## First Responder Team Request

Use `shared` when any one recipient can take ownership.

```json
{
  "channel_id": "ops",
  "recipients": ["alex", "maya", "owner"],
  "decision_resolution": "shared",
  "title": "Who can investigate elevated errors?",
  "summary": "First responder wins and owns the investigation",
  "body_markdown": "Error rate is above threshold for the API. Approve only if you can start now.",
  "fields": [
    { "label": "Service", "value": "api" },
    { "label": "Error rate", "value": "3.8 percent" }
  ],
  "links": [
    { "label": "Dashboard", "url": "https://example.com/dashboards/api-errors" }
  ],
  "options": [
    { "id": "take", "label": "I can take it", "kind": "approve" },
    { "id": "pass", "label": "Cannot take it", "kind": "reject" }
  ],
  "dedupe_key": "ops:api-errors:2026-06-10T16"
}
```

## External Quorum Pattern

Nod v1 does not have server-side quorum. Send a `per_user` request and let the
issuer count acceptable decisions.

```json
{
  "channel_id": "change-approval",
  "recipients": ["owner", "alex", "maya"],
  "decision_resolution": "per_user",
  "title": "Vote on database failover",
  "summary": "Automation proceeds after two approvals",
  "body_markdown": "Approve only if you agree that failover should start now. The issuer will proceed after two approvals or stop after any reject.",
  "fields": [
    { "label": "Cluster", "value": "primary-us-east" },
    { "label": "Required approvals", "value": "2 of 3" }
  ],
  "options": [
    { "id": "approve", "label": "Approve", "kind": "approve" },
    {
      "id": "reject_reason",
      "label": "Reject with reason",
      "kind": "reject_with_text",
      "text_placeholder": "Why should failover wait?"
    }
  ],
  "expires_at": "2026-06-10T17:15:00.000Z"
}
```

Issuer-side rule:

```text
Read or wait for the decision view before acting.
Count decisions where decision.option_kind is "approve".
If approvals >= 2, proceed.
If any decision.option_kind is "reject" or "reject_with_text", stop.
If expires_at passes before quorum, stop or escalate.
```

## Incident Or Ops Request

```json
{
  "channel_id": "ops",
  "title": "Page database owner?",
  "summary": "Replica lag has exceeded the paging threshold",
  "body_markdown": "Replica lag has stayed above 120 seconds for 5 minutes. Paging the database owner may wake someone up.",
  "fields": [
    { "label": "Replica", "value": "db-replica-3" },
    { "label": "Lag", "value": "143 seconds" },
    { "label": "Duration", "value": "5 minutes" }
  ],
  "links": [
    { "label": "Database dashboard", "url": "https://example.com/dashboards/db" },
    { "label": "Paging policy", "url": "https://example.com/runbooks/paging-policy" }
  ],
  "notification": {
    "redact": true,
    "title": "Nod ops request",
    "body": "Open Nod to review an ops decision."
  },
  "options": [
    { "id": "page", "label": "Page owner", "kind": "approve", "destructive": true },
    { "id": "wait", "label": "Wait", "kind": "reject" }
  ],
  "expires_at": "2026-06-10T16:45:00.000Z"
}
```

## Personal Or Family Prompt

```json
{
  "channel_id": "home",
  "recipients": ["owner"],
  "title": "Turn off downstairs lights?",
  "summary": "Motion has been quiet for 30 minutes",
  "body_markdown": "The house automation thinks everyone has gone upstairs. Turning off the downstairs lights will leave the entry lamp on.",
  "fields": [
    { "label": "Last motion", "value": "30 minutes ago" },
    { "label": "Will stay on", "value": "Entry lamp" }
  ],
  "options": [
    { "id": "turn_off", "label": "Turn off", "kind": "approve" },
    { "id": "leave_on", "label": "Leave on", "kind": "reject" }
  ],
  "dedupe_key": "home:lights:downstairs:quiet"
}
```

## Agent Escalation Prompt

```json
{
  "channel_id": "agents",
  "recipients": ["owner"],
  "title": "Agent needs permission to modify billing config",
  "summary": "A code agent is blocked on a sensitive config change",
  "body_markdown": "The agent found a required billing config update. Approving lets it edit the staging config only. Rejecting leaves the task blocked.",
  "fields": [
    { "label": "Repository", "value": "billing-service" },
    { "label": "File", "value": "config/staging.toml" },
    { "label": "Action", "value": "Change staging webhook endpoint" }
  ],
  "links": [
    { "label": "Pull request", "url": "https://example.com/repos/billing-service/pulls/42" }
  ],
  "notification": {
    "redact": true,
    "title": "Agent approval needed",
    "body": "Open Nod to review a requested action."
  },
  "options": [
    { "id": "allow", "label": "Allow", "kind": "approve" },
    {
      "id": "deny_reason",
      "label": "Deny with reason",
      "kind": "reject_with_text",
      "text_placeholder": "Tell the agent what to do instead"
    }
  ],
  "dedupe_key": "agent:billing-service:staging-webhook"
}
```
