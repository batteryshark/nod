# Request and decision signing

Nod issues exactly two signing schemes, numbered so each request digest pairs
with its decision payload:

| Card | Request digest | Decision payload |
| --- | --- | --- |
| Ordinary (no `answer_field`) | `nod-request-v1` | `nod-decision-v1` |
| Value (has `answer_field`) | `nod-request-v2` | `nod-decision-v2` |

`nod-request-v1` is the structured encoder over the immutable card content plus
the salted `recipients_commitment`; `nod-request-v2` is the canonical value
encoder over the same content plus the `answer_field`. `nod-recipients-v2`
(commitment) and both decision payload encodings are unchanged by the numbering
collapse. The canonical implementations and protocol-freeze vectors live in
[`nod-proto/src/signing.rs`](../nod-proto/src/signing.rs) and
[`nod-proto/src/value_signing.rs`](../nod-proto/src/value_signing.rs).

## Historical encoders (verification only)

Already-recorded signatures keep verifying, so the server can still reproduce:

- the original line-snapshot digest (`request_digest_legacy_line_snapshot`),
  tagged `nod-request-v1`, recipients in the clear;
- the structured ordinary digest under its old `nod-request-v2` tag
  (`request_digest_legacy_structured_v2`);
- the value digest under its old `nod-request-v3` tag
  (`value_request_digest_legacy_v3`).

None of these is issued for new cards, and none may be used to sign. A decision
is accepted when its `request_digest` matches the output of any known encoder
for that card's immutable content.

`request.request_digest` at the top level of a full or single-recipient view
stays for backward compatibility only: it is the frozen legacy line-snapshot
value, while `signing.request_digest` carries the collapsed version.

## Client dispatch rule (follow-up client job)

A received signing context is the value variant when the card carries an
`answer_field`, otherwise it is ordinary. Contexts with the pre-collapse tags
`nod-request-v2` without an `answer_field` and `nod-request-v3` with one must
still be accepted for already-issued cards. A context carrying
`recipients_commitment` is always the structured encoder, never the legacy line
snapshot.
