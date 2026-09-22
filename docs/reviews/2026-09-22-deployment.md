# Nod deployment — 2026-09-22

The user authorized deployment to the existing apps host and publication of the
reviewed changes. Four implementation commits were pushed to `main`:

- `61c7584`: coordinated protocol, server, runtime, FFI, TUI, and native adapters.
- `60cb254`: Apple inbox, notification controls, and action recovery.
- `0fe054f`: desktop history, safe rendering, and recoverable controls.
- `e7ee228`: platform CI and uplift documentation.

The deployed server image was built from an exact archive of
`e7ee2289a17fec183c843c9c55097a0f6f87d9b4`. Image digest:
`sha256:b256caff64bc85a3e8bbe811f544016c0b056598390f5a627e184de6f2e5d6cd`.
The application version remains 1.0.1; this is a source-identified deployment,
not a new tagged public release.

## Rollout and preservation

- Built the replacement while the original server continued serving requests.
- Rehearsed migration against a consistent copy of the actual database. The
  canary omitted APNs configuration, disabled callbacks, and exposed its test
  port only through a private SSH tunnel.
- Passed the deployed HTTP/WebSocket smoke, including enrollment and a verified
  signed decision, against that isolated copy. The test was not run amid live
  issuer traffic because its temporary users automatically subscribe to
  `default`.
- Preserved the previous executable as a runnable rollback image, the original
  container configuration, local Compose customization, secret files, and host
  source. Stopped only Nod and captured a consistent complete data-volume backup,
  including audit files and SQLite WAL/SHM files. Verified its checksum.
- Started the immutable production image using the existing data volume,
  interface/port binding, credentials, direct APNs settings, and retention
  policy. The image runs as UID/GID 10001; data ownership was adjusted while
  stopped, with original ownership retained in the backup.
- Fast-forwarded the host checkout without overwriting its Compose customization.
  Retained a pinned deployment override and updated the host's normal local image
  tag to the same verified image. Routine startup no longer compiles the server.

## Verification

The live LAN health and admin endpoints returned HTTP 200. The admin response
includes the hashed-script CSP, no-store policy, no-referrer policy, and nosniff
header. Authenticated summary and activity checks passed; audit health is good.
The container health check is healthy, with zero restarts and no startup errors.

SQLite integrity and foreign-key checks passed. Before/after fingerprints of
existing user identities, device bearer hashes/signing keys, issuer tokens, and
recorded decision receipts matched exactly. Migrated recipient salts and device
preference JSON are valid. No historical push backlog was generated.

The implementation previously passed 240 Rust, 34 Swift, and 42 frontend tests
locally, plus formatting, lint, protocol drift, native Apple builds, and browser
checks. Native CI for the deployed source is tracked by
[run 35754120176](https://github.com/batteryshark/nod/actions/runs/35754120176).
That run records the full native Linux, Windows, macOS, and Apple app pipeline.

## Remaining acceptance and rollback

No live APNs notification was sent by the deployment checks. Physical-device
background/lock-screen actions, accessibility, and signed client distribution
remain separate acceptance work. This rollout upgrades the server; it does not
replace already installed client applications or publish a new app release.

The protected host deployment directory retains the backup, previous image,
verification manifest, and rollback instructions. Rollback must restore the
matching old executable **and** pre-upgrade database/configuration; an old binary
alone does not honor the new tombstone semantics. Preserve any post-upgrade data
before restoring, and reconcile intervening writes. See the [backup and restore
procedure](../deploy.md#back-up-and-restore-a-server).
