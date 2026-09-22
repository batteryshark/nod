#!/usr/bin/env bash
# Build the nod-client-ffi static libraries for Apple platforms, generate the
# UniFFI Swift bindings, and assemble:
#   Frameworks/nod_client_ffiFFI.xcframework   (the embedded Rust client)
#   Sources/NodClientFFI/nod_client_ffi.swift  (the generated Swift wrapper)
#
# This is the bridge that lets NodKit drop its hand-written client logic and use
# nod-client-core (shared with the TUI + desktop). Re-run when nod-client-core's
# exposed FFI surface changes. Both outputs are git-ignored (regenerable);
# requires the Rust toolchain.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APPLE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${APPLE_DIR}/../.." && pwd)"

CRATE="nod-client-ffi"
LIB="libnod_client_ffi.a"
MODULE="nod_client_ffiFFI"
SWIFT="nod_client_ffi.swift"

MAC_ARM="aarch64-apple-darwin"
MAC_X64="x86_64-apple-darwin"
IOS_ARM="aarch64-apple-ios"
SIM_ARM="aarch64-apple-ios-sim"
SIM_X64="x86_64-apple-ios"

# Keep Rust and native dependency objects compatible with the app deployment
# targets; otherwise current Xcode SDKs can stamp assembly with the host SDK version.
export MACOSX_DEPLOYMENT_TARGET=14.0
export IPHONEOS_DEPLOYMENT_TARGET=17.0

cd "${REPO_ROOT}"

echo "==> Building ${CRATE} static libs (release)"
for target in "${MAC_ARM}" "${MAC_X64}" "${IOS_ARM}" "${SIM_ARM}" "${SIM_X64}"; do
  rustup target add "${target}" >/dev/null
  cargo build -p "${CRATE}" --release --target "${target}"
done

echo "==> Generating Swift bindings"
cargo build -p "${CRATE}" --release
GEN_DIR="$(mktemp -d)"
cargo run -q -p "${CRATE}" --bin uniffi-bindgen -- generate \
  --library "${REPO_ROOT}/target/release/libnod_client_ffi.dylib" \
  --language swift --out-dir "${GEN_DIR}"

HDR_DIR="${GEN_DIR}/headers"
mkdir -p "${HDR_DIR}"
cp "${GEN_DIR}/${MODULE}.h" "${HDR_DIR}/"
cp "${GEN_DIR}/${MODULE}.modulemap" "${HDR_DIR}/module.modulemap"

echo "==> Fusing universal static libs"
STAGE="$(mktemp -d)"
mkdir -p "${STAGE}/macos" "${STAGE}/ios" "${STAGE}/ios-sim"
lipo -create "target/${MAC_ARM}/release/${LIB}" "target/${MAC_X64}/release/${LIB}" \
  -output "${STAGE}/macos/${LIB}"
cp "target/${IOS_ARM}/release/${LIB}" "${STAGE}/ios/${LIB}"
lipo -create "target/${SIM_ARM}/release/${LIB}" "target/${SIM_X64}/release/${LIB}" \
  -output "${STAGE}/ios-sim/${LIB}"

echo "==> Assembling xcframework"
XCF="${APPLE_DIR}/Frameworks/${MODULE}.xcframework"
rm -rf "${XCF}"
mkdir -p "${APPLE_DIR}/Frameworks"
xcodebuild -create-xcframework \
  -library "${STAGE}/macos/${LIB}" -headers "${HDR_DIR}" \
  -library "${STAGE}/ios/${LIB}" -headers "${HDR_DIR}" \
  -library "${STAGE}/ios-sim/${LIB}" -headers "${HDR_DIR}" \
  -output "${XCF}"

echo "==> Placing generated Swift wrapper"
DEST="${APPLE_DIR}/Sources/NodClientFFI"
mkdir -p "${DEST}"
cp "${GEN_DIR}/${SWIFT}" "${DEST}/${SWIFT}"

rm -rf "${GEN_DIR}" "${STAGE}"
echo "==> Done: ${XCF}"
