# Nod project review — 2026-09-22

Reviewed checkout: `3e3d086` (workspace version 1.0.1). This is a review and proposed improvement backlog; application code has not been changed.

**Overall assessment.** Nod has a useful product concept and a sensible foundation: a small self-hosted server, SQLite, one shared Rust client runtime, native platform shells, and a shared signing contract. The highest-value next release would make existing workflows dependable. There are significant defects in authorization lifetime, request creation, cross-device synchronization, signing projected requests, and native notification actions. These matter more than reorganizing the architecture or adding more platforms.

The user experience also needs to explain what happened: whether the inbox is current, whether an approval reached the server, why a notification failed, and who resolved a request. Today several operations can fail while the UI looks healthy or dismisses the user's input.

**Scope and confidence.** Reviewed server/API/storage, APNs relay, shared protocol and client runtime, Apple clients/FFI, Tauri/React desktop, TUI, admin UI, onboarding, CI and release scripts. Inspected the committed screenshots as visual references. Findings distinguish executed reproductions, direct source evidence, and product recommendations. This was not a live Windows/iOS acceptance run, a production penetration test, or a load benchmark. Native behavior needing hardware verification is called out explicitly.

**Verification performed.**

| Check | Result |
| --- | --- |
| Rust workspace tests, locked/offline dependencies | 164 passed; 1 deployed-instance test intentionally ignored |
| Swift tests | 20 passed, using the existing generated FFI framework |
| Desktop frontend tests | 29 passed |
| TypeScript type checking | Passed |
| Rust formatting and workspace/all-target Clippy with warnings denied | Passed |
| Type projection drift check | Passed: 15 interfaces, 6 enums |
| Isolated HTTP/WebSocket reproductions | Partial create, post-revocation reads, mutable digest, callback latency confirmed |
| Source-importing reducer harness | Resolved event leaves request pending; profile ID collision confirmed |

Initial execution hit a moved-directory Tauri cache and sandbox restrictions on loopback test sockets/Swift caches; reruns succeeded after rebuilding the affected cache and allowing the test processes host access. Those were environment problems, not Nod failures. Swift linking warned that some objects in the existing FFI archive target macOS 26.5 while the package targets 14.0. This does not establish a release-binary defect, but oldest-supported-OS validation should include a freshly rebuilt framework. The deployed-instance smoke test was not run against production. No real user data or live deployment was changed.

**Priorities and effort.** P1 means fix before expanding use or relying on Nod for consequential approvals. P2 means a significant reliability, usability, or operational improvement. Effort is relative: S is a localized change and regression test, M spans a few components, L changes a protocol or several platform integrations. These are sizing judgments, not delivery commitments.

| Order | Work | Priority | Effort |
| --- | --- | --- | --- |
| 1 | Replace Windows shell-based URL opening | P1 | S |
| 2 | Enforce revocation on existing sockets | P1 | M |
| 3 | Apply every request state update and reconcile reconnects | P1 | M |
| 4 | Reconcile the signed snapshot with recipient privacy projections | P1 | L |
| 5 | Make creates and per-user decisions transactional | P1 | M |
| 6 | Fetch and route notification actions by server/channel/request | P1 | M–L |
| 7 | Preserve immutable decision evidence after account/device deletion | P1 | M–L |
| 8 | Fix native action contracts, failure recovery, and TUI interaction | P2 | M |
| 9 | Repair quickstart/examples and add guided enrollment | P2 | S–M |
| 10 | Add notification health, searchable history, and faster navigation | P2 | M–L |

**1. Windows links cross into a command shell — P1, high confidence from source.**

The Windows opener passes request-controlled link text to `cmd /C start "" URL`. A user clicking a crafted request link reaches a shell interpretation boundary. The server validates callback schemes but does not validate these card links. Replace the shell with an OS URL-opening API and explicitly allow supported schemes; retain HTTP for legitimate private/self-hosted deployments. Do not try to fix this by adding ad hoc shell escaping.

Evidence: [Windows opener](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/external_url.rs#L3), [link click](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/RequestDetail.tsx#L63), [server validation](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/validation.rs#L9). Rust documents the special malicious-argument risk of `cmd.exe` in its [Command API](https://doc.rust-lang.org/std/process/struct.Command.html#method.arg). No Windows exploit was executed. Verify accepted URLs open correctly and unsupported schemes/metacharacters cannot launch another command.

**2. Device revocation leaves an existing WebSocket authorized — P1, reproduced.**

Authentication happens at upgrade. The forwarding loop never closes the connection when its device is revoked. In an isolated run, revocation returned 200 and subsequent HTTP returned 401, but the old socket still received a newly created request containing its private summary. This is continued read access for a previously authorized connection, not an unauthenticated bypass or continued ability to submit decisions through HTTP.

Evidence: [upgrade and forwarding](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/api/sync_socket.rs#L22). Cancel connections on revocation and enforce authorization lifetime on the server, including deletion/revocation races and missed broadcasts. A cooperative client logging out is insufficient. Regression test with an intentionally noncooperative socket that stays open after revocation.

**3. Resolved, cancelled, and expired events do not update the shared client — P1, reproduced.**

The reducer evaluates `kind == created && apply_request_update(...)`. Short-circuit evaluation skips the state update for every other event kind. A resolution from another device leaves the card pending and the count inflated; local submission or manual refresh can mask the defect. A harness importing the actual reducer produced `Pending; pending={"c": 1}` after a resolved envelope.

Evidence: [state reducer](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/state.rs#L113). Apply the update first, then decide whether to notify. Test created → resolved/expired/cancelled transitions, repeat delivery, off-screen channels, and per-user outcomes through the envelope entrypoint, not only its helper methods.

**4. Reconnecting does not mean the inbox is current — P1, high confidence.**

The shared runtime reconnects its socket and reports connected without fetching missed state or replaying a cursor. Requests arriving during an outage may never appear. Desktop ignores `ResyncRequired`; the TUI asks the user to press R. This is distinct from finding 3: even a correct event reducer cannot apply events it never received.

Evidence: [connection loop](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/runtime/sync.rs#L95), [desktop message handling](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/app/useDesktopClient.ts#L283), [TUI recovery](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/app.rs#L190). Own reconciliation inside the shared runtime, with race-safe snapshot/event ordering. Show connecting, reconciling, current, and offline states; include last successful sync. Add liveness detection and capped reconnect backoff with jitter. A snapshot solution is adequate at this scale; a durable broker is not required.

**5. Multi-recipient requests fail the real client's digest check — P1, high confidence, independently reviewed.**

The digest includes all recipients. The server stamps that digest and then filters recipients down to the viewing user. The client recomputes from the filtered content and refuses to sign. This affects both shared and per-user requests received through HTTP and targeted WebSocket projections. Server tests that manually sign the supplied digest bypass the real client's stricter path.

Evidence: [client verification](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/signing.rs#L96), [server projection](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/read.rs#L146), [digest contract](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/nod-proto/src/signing.rs#L99). Define a representation that allows the client to verify the approved content while preserving intended recipient privacy. Version a changed signing contract and preserve old verification vectors. Simply removing content verification or exposing all recipients would silently discard an existing safeguard. Acceptance test: two users, both resolution modes, HTTP reload and WebSocket delivery, actual client signer, successful server verification.

**6. Failed creates persist partial requests; per-user decisions have a terminal-state race — P1.**

Creation commits the parent and recipients before validating/inserting every option. Reproduction: an invalid Reject option returned 400 but left a pending Approve-only request. A corrected retry with the same dedupe key returned that damaged request with `deduped: true`. Validate all options first and insert the complete graph in one transaction. Concurrent dedupe retries must converge on one complete request.

Evidence: [create sequence](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/create.rs#L58), [late validation](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/create.rs#L125).

A related static finding exists in [per-user decisions](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/decisions.rs#L35): pending state is checked before a later unconditional decision insert. Cancellation/expiry can win between these operations while the decision is still recorded. Shared decisions already have a conditional update, so do not generalize this flaw to that branch. Make eligibility, nonce use, decision insertion, and aggregate status atomic; test cancellation versus approval with controlled concurrency. The race was not dynamically reproduced.

**7. Notification actions depend on whichever server/channel happens to be open — P1, high confidence.**

Signing requires the request in the currently selected channel's memory. Desktop activation selects an ID without loading its channel; Apple quick actions submit immediately. iOS disconnects WebSocket sync in the background, so a newly pushed request is especially likely to be absent. Payloads lack server identity. Clicking a notification from another channel/server can fail with “request … is not loaded” or use the wrong connection context.

Evidence: [signing lookup](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/runtime/session.rs#L95), [desktop activation](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/runtime_messages.rs#L117), [Apple submission](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NodStore.swift#L561), [iOS lifecycle](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/NodIOS/NodIOSApp.swift#L48).

Introduce one server-scoped operation that waits for startup, fetches the authoritative request, checks current availability, and routes or submits the actual option. Test cold launch, background delivery, channel B while viewing A, server switching, and a request resolved elsewhere. Surface an actionable failure rather than silently consuming the notification. On iOS, also navigate to the request itself: [the current tap handler](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/NodRootViews.swift#L114) discards its request ID and opens only the channel.

**8. Removing accounts/devices changes or removes historical verification material — P1, partly reproduced.**

User deletion cascades into request recipients and per-user decisions. Deleting a recipient in an isolated run changed the digest of an existing request. Device revocation deletes stored verification public keys, while decision records retain a key ID/signature rather than the complete key material. These operations undermine later reconstruction and independent verification of the original decision.

Evidence: [recipient/decision schema](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/schema.sql#L151), [user deletion](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/users.rs#L151), [device deletion](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/devices.rs#L98), [signature record](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/nod-proto/src/decision.rs#L32). Preserve immutable recipient/decision snapshots and non-secret historical public keys; revoke credentials separately. Tombstones are enough. Test signature verification after revocation, account removal, and backup/restore.

**9. Native notification action rendering is incomplete — P2, high confidence from source/schema.**

Apple registers fixed `approve`/`reject` IDs and selects those categories for nonempty option lists, even when a request uses other IDs or custom choices. Windows emits `<options><option>` instead of the documented `<actions><action>` schema, and its activation path ignores the intended open-detail behavior for options requiring text. These are independently fixable issues beyond the routing problem above.

Evidence: [Apple categories](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NotificationController.swift#L92), [Windows XML](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/notifier/windows_toast.rs#L15), [Windows activation](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/notifier/platform/windows.rs#L49), [Microsoft action schema](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-actions). Preserve actual option IDs, labels, text/foreground behavior; use “Open Nod” where an OS cannot represent the choices safely. Verify the native notification end to end; substring assertions on generated XML are insufficient.

**10. Completed requests leave obsolete OS notifications — P2.**

Apple handles removal by updating an in-memory dedup set, without removing delivered/pending notifications. Windows' removal function is a no-op. Old approvals remain actionable after another device has resolved them.

Evidence: [Apple removal](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NodStore.swift#L205), [Windows removal](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/notifier/platform/windows.rs#L68). Use server-scoped notification identifiers/tags, remove terminal requests, and reconcile OS notification history after refresh. Include cancellation, expiry, logout, and forgotten servers.

**11. Failed actions lose input or prevent retry — P2.**

Apple's reply sheet closes and clears text even when submission failed, because the store catches errors and returns no success result. Enrollment for a second server can dismiss after failure because the app was already registered to the first. Informational auto-dismiss records its dedup key before attempting submission and leaves it recorded after failure. Desktop action controls also lack operation-specific in-flight state.

Evidence: [reply sheet](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/RequestDetailView.swift#L92), [registration](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/RegistrationView.swift#L30), [auto-dismiss](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NodStore.swift#L355), [desktop actions](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/RequestDetail.tsx#L80). Return explicit operation outcomes, preserve drafts, dismiss only on success, and separate in-flight from successfully completed IDs. For ambiguous network failures, fetch authoritative state before offering a retry. Do not queue stale approvals blindly for later submission.

**12. TUI navigation and text entry block ordinary use — P2.**

Typing lowercase `q` in notes, filters, or device rename closes the modal. Lists have no selected-item viewport, and request detail has no scroll offset; content longer than the terminal cannot be fully inspected. Custom options display an `n` hint, but that key selects only a text-required option.

Evidence: [close shortcut](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/app.rs#L557), [text input](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/app/option_text.rs#L39), [list rendering](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/ui/panes.rs#L124), [detail rendering](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/ui/panes.rs#L198), [custom action lookup](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-tui/src/app.rs#L504). Reserve Escape for cancelling editable inputs, add list/detail scrolling and Page Up/Down/Home/End, and provide a selectable action list. Test ordinary words containing `q`, long cards, and more rows than screen height.

**13. Preserve every valid decision option in the UI — P2.**

Apple's paired approval layout selects the first option of each kind, then excludes all options of those kinds from the general list. Two valid approval choices can therefore hide the second. Some issuer labels become generic “Approve” or “Reject.” Desktop chooses the check/cross icon from `destructive` rather than option meaning, so a Reject without that flag can show a checkmark (also visible in the committed screenshot).

Evidence: [Apple option selection](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/RequestDetailView.swift#L181), [remaining choices](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/RequestDetailView.swift#L236), [desktop icon](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/RequestDetail.tsx#L83). Preserve each ID and label; use paired shortcuts only for compatible conventional sets. Keep semantic choice, destructive styling, and foreground behavior separate.

**14. The front-door documentation has reproducible onboarding traps — P2, small fixes.**

The README generates an admin token only in the server process environment, then asks the user to log in with a value it neither displays nor saves. The main JSON example contains `priority`, which the strict create API does not accept. New users following the prominent example will fail before reaching Nod's strengths.

Evidence: [quickstart](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/README.md#L24), [unsupported priority](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/README.md#L123), [strict API input](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/api/requests.rs#L26). Give a copyable setup flow that securely saves the token and tells users how to retrieve it; make examples executable contract fixtures. Explain that a phone's `localhost` is the phone, then lead into the private HTTPS/Tailscale instructions. Keep claimed features and examples aligned: priority is currently a proposal, not a working capability.

**15. Notification privacy should work consistently on every platform — P2 safety improvement.**

Local Apple, Windows, and Linux notification rendering uses full request title/body rather than the notification preview overrides/redaction hints. Apple may also download and attach the request image. A user can see generic APNs text on the phone and detailed content on a desktop lock screen for the same request.

Evidence: [Apple content](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NotificationController.swift#L135), [Windows content](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/src/notifier/windows_toast.rs#L6). **Current API documentation explicitly scopes `notification.redact` to APNs**, so this is not a breach of that documented guarantee. It is a significant gap in the user-facing privacy model. Share one safe preview projection, suppress attachments when redacted, and add a local “hide notification content” override. Clarify preview privacy versus server-side content visibility.

**16. Push delivery needs health information, bounded work, and retries — P2.**

A title-only request is valid but produces an empty push body that relay validation rejects. Push sends are detached tasks with failures only logged; an unmatched provider route returns success without a delivery attempt. The direct APNs HTTP client has no configured timeout. Users cannot distinguish “configured” from “working.”

Evidence: [summary fallback](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/create.rs#L44), [push projection](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/apns_relay.rs#L239), [body validation](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-apns-relay/src/relay.rs#L51), [detached delivery](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/services/requests.rs#L162), [direct client](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-apns-relay/src/apns/client.rs#L32).

Add a safe default body, per-device last attempt/success/error, transport timeouts, bounded concurrency, transient retries with backoff, and invalid-token handling. “Test this device” should report the actual stage reached; provider acceptance is not proof the person saw it. Make foreground fallback a per-device capability rather than only a global server setting. macOS already explicitly permits local alerts even with APNs configured; preserve that behavior.

Optional Apple images should not hold up the essential alert. [Image attachment loading](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Sources/NodKit/NotificationController.swift#L144) precedes posting, and notifications are processed sequentially. Set short time/size limits, validate responses, clean temporary files, and make external media loading controllable.

**17. Best-effort callbacks should not block an already-committed approval — P2, reproduced.**

A callback is awaited after the decision commits and before the device receives its response. A two-second local callback delay produced a 2.011-second decision response; the configured timeout permits a longer delay. Users can experience “still submitting” even though the decision already took effect.

Evidence: [decision response path](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/services/requests.rs#L83), [callback send](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/services/requests.rs#L173). Return the committed decision promptly and perform callbacks in bounded background work. Use a durable outbox only if reliable retries become part of the contract. Retain the documented rule: unsigned callbacks are wake-up hints; authenticated read/wait APIs establish the decision.

Callback destinations also need an operator-controlled trust policy. An authorized issuer can make Nod contact arbitrary reachable HTTP(S) services after a decision; error responses are read/logged without an explicit size cap. Allow configured origins, control redirects, and bound response/log bytes. Private-network callbacks are useful, so a blanket private-address ban would break legitimate deployments. This is hardening for issuer trust boundaries, not evidence of an unauthenticated public exploit.

**18. Retention, subscription, and audit semantics need consistent user-visible rules — P2.**

Retention deletes requests by creation age, including pending requests with no expiry, and emits no terminal event. Listing also hides these old pending requests. Explicit recipients bypass subscription checks at create time, while HTTP lists/APNs require subscription and WebSocket delivery uses recipient membership. A request can appear live and vanish after refresh. These are policy choices that need one consistent implementation.

Evidence: [retention deletion](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/maintenance.rs#L62), [inbox eligibility](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/read.rs#L27), [explicit recipients](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/create.rs#L177). Prefer retaining pending work and aging history from terminal status, or explicitly expire pending work before removal. Decide whether explicit targeting overrides subscription and enforce it across every transport.

The [audit logger](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/audit.rs#L26) appends indefinitely to one file and reports write failures only to logs. SQLite retention does not erase that file. Specify audit retention separately, add rotation/archive and degraded-health visibility, and distinguish a best-effort log from a guaranteed durable evidence record. Provide a tested backup/restore procedure that includes the database, verification material, and intended audit history.

**19. Multi-server identity and local persistence need small reliability fixes — P2.**

Profile IDs replace URL punctuation with hyphens and truncate. `https://nod.example.com/a-b` and `https://nod.example.com/a/b` produce the same ID; enrollment then upserts credentials/profile under that key. Forgetting a server disconnects sync without reconnecting the remaining selected profile. The persisted config is overwritten directly, so an interrupted write can damage the only copy.

Evidence: [profile identity](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/api.rs#L363), [credential/profile upsert](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/runtime/workflows.rs#L55), [forget workflow](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/runtime/workflows.rs#L106), [config write](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/store.rs#L82). Use a stable collision-resistant ID from a correctly normalized URL, with migration; reconnect the replacement profile; use atomic write/rename and coordinate concurrent desktop/TUI writers. Keep OS credential storage as the default and restrictive file permissions for explicit insecure mode.

**20. Self-hosted admin code should not need an external executable dependency — P2 hardening.**

The admin page loads its emoji picker as a module from a public CDN. That code runs in the admin page's authority and can observe the login form or make authenticated same-origin requests. Version pinning helps reproducibility but is not isolation. This is a trust dependency, not evidence that the dependency is malicious.

Evidence: [external module](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/assets/admin.html#L555). Bundle the picker/assets or use a simple local alternative; keep administration functional offline and introduce an appropriate CSP. When adding desktop Markdown rendering, likewise use a constrained renderer and revisit the [currently disabled CSP](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src-tauri/tauri.conf.json#L23).

**The QOL roadmap I would prioritize.** These are product recommendations beyond repairing defects above.

| Improvement | Concrete user benefit and proposed behavior | Effort |
| --- | --- | --- |
| A single pending inbox | See all pending work for the active server, with channel labels, search, status/date filters and keyboard navigation. Add cross-server aggregation only with explicit server identity and separate connections. | M |
| Useful decision receipts | Show chosen option, actor, notes, decision time, server/channel, shared versus per-user progress, expiry/countdown, and a copyable request ID. Label cryptographic verification accurately; do not imply it proves the server is honest. Desktop currently shows only status/content after resolution. | M |
| Searchable, paged history | Fetch older results per channel/query; show retention limits. A global newest-500 cap currently lets one busy channel crowd another channel's history out. | M |
| Guided enrollment and diagnostics | Explain where to obtain a code, offer a short-lived QR/deep link, validate the server address, show progress/retry, then test notification permission and delivery. Treat enrollment links as secrets and avoid telemetry/logging them. | M |
| Recoverable actions | Keep notes after failure; show sending/sent/conflict/outcome-unknown states; offer authoritative refresh and retry. Preserve navigation context after an action. | M |
| Notification controls | Per-channel local mute/snooze and optional quiet hours, content hiding, working sound previews, permission recovery, and a test result. Snoozing/muting must not silently change eligibility or approve work. | M |
| More readable request cards | Safe Markdown on desktop (currently a literal preformatted block), readable long links, copy buttons, expandable fields/images, visible destinations. Choose approve/reject iconography from semantics. | S–M |
| Accessible navigation and dialogs | Desktop dialog roles, focus trapping/return, Escape handling, named icon buttons and error announcements; Apple VoiceOver/Dynamic Type enrollment/action checks; TUI scrolling and discoverable shortcuts. | M |
| Predictable background behavior | Expose launch-at-login/background controls; verify close/minimize/reopen. macOS “Open Nod” currently activates the app then calls a Settings selector despite no Settings scene; give the inbox a window ID and test reopening after closing it. | S–M |
| Targeted protection for destructive settings | Confirm device revocation/forgetting with specific consequences; allow undo for reversible clearing. Avoid adding a confirmation dialog to every routine approval. | S |
| Admin delivery/activity view | Show recent requests, per-user results, per-device delivery attempts and errors, pending age, and token use. Extend the existing test-request screen into a verifiable round trip. | M |

Evidence for these gaps: [desktop detail](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/RequestDetail.tsx#L44), [desktop list](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/RequestList.tsx#L15), [settings dialog](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-desktop/src/components/SettingsDialog.tsx#L30), [Apple enrollment](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/Shared/RegistrationView.swift#L75), [macOS reopening](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Apps/NodMac/NodMacApp.swift#L34), [admin test result](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/assets/admin.html#L1934).

Viewing informational requests currently auto-dismisses them. That is intentional behavior, not an accidental bug. Consider separate “read” and “acknowledged” semantics only if real workflows need them; preserve a lightweight notification mode.

**Performance work with the clearest payoff.**

The inbox path selects IDs, then loads each request, recipients, decisions, and options sequentially. Five hundred handled items imply roughly 2,000 dependent row-loading queries plus selection/authentication work; pending items are unbounded. [List hydration](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/read.rs#L110) and [related-row reads](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/server/nod-server/crates/nod-server/src/db/requests/rows.rs#L17) are the first places to optimize. Batch related rows and assemble by ID; paginate history without silently dropping pending work.

Selecting a channel triggers the [full refresh](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-client-core/src/runtime/workflows.rs#L274), which sequentially fetches user, devices, channels, and all recent requests. Keep an appropriate per-server cache and filter locally; refresh independent data concurrently where consistency permits. Avoid rewriting the whole state store merely for style. Throttle last-seen writes, reuse connections, and bound push/media work before considering infrastructure changes.

Measure cold/warm inbox latency, channel-switch latency, request-to-alert time, decision-response latency, database query counts, and memory for 100/1,000/10,000-item fixtures. No latency or capacity claim in this report is based on a load benchmark; the callback measurement is the isolated two-second reproduction described above.

**Engineering quality: test complete journeys and align the docs.**

The current green suite is useful but leaves gaps between well-tested components. Add a compact integration matrix around real shared-client behavior: multi-recipient signing, events resolving on a second device, disconnect/create/reconnect, noncooperative revoked sockets, create rollback/dedupe races, cancellation versus per-user approval, and notification activation outside the current channel. Add platform tests for cold/background actions, actual Windows toast schema, preserved drafts, and long TUI viewports. These have more value than tests duplicating implementation details.

Run the existing type drift check in CI; [the current frontend CI step](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/.github/workflows/ci.yml#L50) runs typecheck/tests but omits it. Strengthen drift protection with serialized contract fixtures: field-name agreement alone does not verify field types, optionality, or semantics. Compile the iOS application target in CI as well as the Swift package; [Package.swift excludes NodIOS from the executable target](https://github.com/batteryshark/nod/blob/3e3d086063086e1c2ed05ff7aaedd9111a2de2d5/client/nod-apple/Package.swift#L48). Verify Windows adapters on Windows, since macOS tests cannot exercise their conditional code. Make the release gate prepare/rebuild required TLS/FFI fixtures explicitly and gate publication on results for the release commit.

Documentation has accumulated contradictory historical state. For example, the desktop PROJECT notes say no CI exists, while CI now exists; the architecture notes describe generated Swift/TypeScript contracts while the generation script documents intentional hand-written projections. Update the canonical architecture/development notes to the current model and keep completed task history elsewhere. Turn README/API examples into contract fixtures so unsupported fields cannot return unnoticed.

In the applied review taxonomy, the major code problems are expected behavior/boundary correctness (G2/G3) and missing regression coverage (T5/T6), rather than naming or arbitrary function-length violations. Thin shells, focused workflows, and explicit protocol models are worth preserving. Avoid a broad cleanup that would make a safety repair harder to review.

**What I would keep.** SQLite and a process-local event bus are appropriate for the documented single-node deployment. The shared protocol/signing implementation, frozen canonicalization vectors, hashed bearer tokens, OS credential storage, Apple hardware-key boundary without silent fallback, mTLS relay and bundle pinning, typed frontend commands, and separated TUI reducer/renderer are good choices. A rewrite, message broker, microservices, or new UI framework would not solve the defects found here. Unsigned Windows distribution is a documented tradeoff; signing/installer/update improvements are useful later, but correctness and notification reliability come first.

**Suggested delivery order.** First, repair the P1 authorization, transaction, signing, synchronization, and notification-routing paths with end-to-end regressions. In parallel, take the small onboarding/TUI fixes. Next, deliver recovery states, consistent preview privacy, native notification cleanup, receipts, and delivery diagnostics. Then improve all-channel search/history, accessibility, and measured performance. Defer broader refactors and infrastructure expansion until those journeys are dependable.

Temporary reproduction artifacts from this review: `/private/tmp/nod-review-repro.py`, `/tmp/nod-review-client-repro.rs`, and `/tmp/nod-review-*-test-host.log`. They use synthetic data and are not required runtime dependencies. No fixes, commits, releases, or deployment changes were made as part of this review.
