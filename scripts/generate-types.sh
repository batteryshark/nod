#!/usr/bin/env bash
# Generate a TypeScript projection of the Rust source of truth (nod-proto wire
# types + nod-client-core view types) via typeshare, as a DRIFT-DIFF REFERENCE.
# Output -> client/nod-desktop/src/dto/generated.ts (ignored, never imported).
# The frontend maintains models.ts because typeshare treats serde defaults as
# optional. npm run drift-check validates field names and enum values, while
# typecheck and integration tests cover runtime interpretation. The drift gate
# runs in CI. See docs/architecture.md for the full contract boundary.
# Requires: cargo install typeshare-cli --version 1.13.4 --locked
#
# Swift is intentionally not generated: typeshare emits snake_case, which clashes
# with NodKit's camelCase Codable models, and NodKit's security-critical crypto is
# already shared from Rust via UniFFI (nod-client-ffi). NodKit's wire types stay
# hand-written and are verified by its build + tests; nod-proto is the contract.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

TS_OUT="client/nod-desktop/src/dto/generated.ts"
mkdir -p "$(dirname "${TS_OUT}")"

echo "==> TypeScript (nod-proto + nod-client-core view types) -> ${TS_OUT}"
typeshare nod-proto/src client/nod-client-core/src --lang typescript --output-file "${TS_OUT}"

echo "==> Done"
