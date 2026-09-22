# Apple Client Setup

The package in `client/nod-apple` is intentionally SwiftUI-native UI over the
shared Rust client. The client logic (HTTP API, websocket sync, state, decision
orchestration) lives in `nod-client-core` and is driven through the
`NodClientFFI` UniFFI runtime; Swift implements only the native adapters. It
contains:

- `NodKit`: the runtime bridge (`NodRuntimeClient`/`NodRuntimeState`), the SwiftUI-facing `NodStore` facade, and the native adapters — Secure Enclave decision signing, App Attest, UserNotifications/APNs registration, markdown rendering.
- `Sources/NodClientFFI` + `Frameworks/`: the generated UniFFI wrapper and `nod_client_ffiFFI.xcframework` (git-ignored; rebuild with `scripts/build-nod-client-ffi.sh` whenever the Rust side changes).
- `Apps/NodMac`: macOS menu bar/window app shell.
- `Apps/NodIOS`: iOS app shell.
- `Nod.xcodeproj`: Xcode app targets for iOS and macOS, linked to the local `NodKit` package.
- A SwiftPM `NodMac` executable target that compiles the macOS app for quick local checks.

## Xcode

Full Xcode is installed at:

```bash
/Applications/Xcode.app
```

This machine currently selects Command Line Tools for command-line builds. When you are ready to build/sign the iOS app, switch Xcode selection:

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

Open the generated Xcode project:

```bash
open client/nod-apple/Nod.xcodeproj
```

The iOS target is configured with:

- Bundle ID: `com.batteryshark.Boop`
- Automatic signing
- `aps-environment = development` for Debug and `production` for Release/TestFlight
- `UIBackgroundModes = remote-notification`

The macOS target is configured as `com.batteryshark.NodMac`.

## Pairing and Servers

The Apple clients support multiple Nod servers. Pair each server with a short-lived pairing code, and the token is stored in Keychain under that server profile.

The apps do not ship with a default server URL. Enter the URL from your Nod server along with an enrollment code from the server's admin panel.

Pairing codes are entered through fixed uppercase boxes to avoid autocorrect and whitespace issues. After pairing, the app shows servers first, then subscribed channels, then the request list for the selected channel. Channel visibility is controlled from the Subscriptions sheet.

Build the local runnable macOS app bundle with the canonical script:

```bash
./scripts/build-macos-app
```

That script refreshes:

```bash
build/DerivedData/Build/Products/Release/Nod.app
```

It also stamps the macOS bundle with a timestamped build number, so the app's
About/build info reads like `1.0 (202606101430)`. Override it when needed:

```bash
NOD_MAC_MARKETING_VERSION=1.0 NOD_MAC_BUILD_NUMBER=202606101430 ./scripts/build-macos-app
```

Do not use `swift build --product NodMac` when you need the app bundle; it
only builds the SwiftPM executable at `.build/.../NodMac`.

Compile-check the shared client and macOS app channel without changing global Xcode selection:

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  CLANG_MODULE_CACHE_PATH=/private/tmp/nod-clang-cache \
  swift build --package-path client/nod-apple --scratch-path /private/tmp/nod-swiftpm --product NodMac
```

Compile-check the iOS Xcode app target without changing global Xcode selection:

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcodebuild -project client/nod-apple/Nod.xcodeproj -scheme NodIOS -configuration Debug -destination generic/platform=iOS -allowProvisioningUpdates build
```

## Release builds (Developer ID + notarization)

The publishable macOS artifact is a Developer ID-signed, notarized, stapled
DMG produced by `scripts/release-macos`. Dev builds from
`scripts/build-macos-app` (Apple Development or ad-hoc signed) are testing
artifacts only — Gatekeeper blocks them on other machines.

One-time setup:

1. Create a **Developer ID Application** certificate: Xcode → Settings →
   Accounts → Manage Certificates → "+" → Developer ID Application (or via
   developer.apple.com → Certificates). Confirm with
   `security find-identity -v -p codesigning | grep "Developer ID"`.
2. Store notary credentials once, using an
   [app-specific password](https://support.apple.com/102654):

   ```bash
   xcrun notarytool store-credentials nod-notary \
     --apple-id you@example.com --team-id TEAMID
   ```

Then each release is one command:

```bash
scripts/release-macos 1.0.0
```

It rebuilds the FFI + app, signs inside-out with the hardened runtime,
notarizes and staples both the app and the DMG, verifies with `spctl`, and
writes `build/release/Nod-<version>.dmg` plus its SHA-256. The app is signed
without entitlements (same as the verified dev build): the App Attest
entitlement is restricted and would require provisioning-profile machinery,
and macOS enrollment treats attestation as best-effort.

## TestFlight

Apple requires an App Store Connect app record before uploaded builds can
appear in TestFlight. Create the app with bundle ID `com.batteryshark.Boop`,
then use a team App Store Connect API key to archive and upload from the
command line. A team key is preferred because the script allows Xcode to create
or update signing assets when automatic signing needs them.

Required local environment:

```bash
export APP_STORE_CONNECT_API_KEY_ID="..."
export APP_STORE_CONNECT_API_ISSUER_ID="..."
export APP_STORE_CONNECT_API_KEY_PATH="/path/to/AuthKey_....p8"
```

Then upload:

```bash
scripts/testflight-ios
```

By default, the script marks the upload as internal-TestFlight-only and uses a
UTC timestamp build number so each upload is accepted without editing project
files. To archive without uploading:

```bash
scripts/testflight-ios --archive-only
```

To allow the uploaded build to be distributed to external TestFlight testers
later:

```bash
scripts/testflight-ios --external
```

The script also accepts these optional overrides:

```bash
export NOD_MARKETING_VERSION="1.0"
export NOD_BUILD_NUMBER="202605271715"
export NOD_APPLE_TEAM_ID="Y734633UDM"
export NOD_IOS_BUNDLE_ID="com.batteryshark.Boop"
```

TestFlight builds use production APNs tokens, so the Nod server that receives
paired TestFlight devices should use the production Apple APNs provider:

```bash
NOD_APPLE_APNS_ENVIRONMENT=production
NOD_APPLE_APNS_BUNDLE_ID=com.batteryshark.Boop
```

When the Nod server reports `notification_delivery.mode = "websocket"`, the iOS
client presents WebSocket `created` requests as local notifications while the app
is active and connected. This is a foreground fallback only; background and
lock-screen delivery still require APNs. Direct APNs and notification relay
routes are transparent to Apple clients and both appear as `mode = "push"`.

## Notification Categories

The clients register:

- `NOD_DEFAULT`
- `NOD_APPROVAL`
- `NOD_APPROVAL_TEXT`

APNs uses approval categories only when the request has the exact supported
option IDs and labels; other requests use Open Nod. Local notifications register
request-specific actions for up to three options; longer or redacted requests
open the app. Responses require device authentication. Notifications carry the
originating device/server identity so responding does not depend on the server
currently selected in the app.

Local alerts use the same safe title/body projection as APNs. In Settings,
Alerts on This Device saves preferences for this device on the selected server:
hide content, mute individual channels, and pause alerts for 1, 8, or 24 hours.
These preferences apply to server push and local alerts while requests remain in
the inbox. Use system Focus for scheduled quiet hours. The notification test
checks this device's presentation, not server push delivery. iOS settings show
when background push is unavailable and only foreground sync is configured.

## Inbox and setup

Choose All requests to work across the selected server's channels. Search filters
the loaded inbox; Search History queries the server and can load older pages.
Connection status shows whether the view is current and when it last updated.
Decision details show the recorded actor, timestamp and server verification result.

Informational requests acknowledge on open by default. Turn off the Reading
preference to use an explicit Acknowledge action; for shared requests either
action completes the item for everyone. Response drafts remain in memory when navigating or
retrying a failed send, and are cleared after successful submission. Text-capable
actions collect optional notes; an empty response remains valid.

A setup link uses `nod://enroll?server=<encoded URL>&code=<8-character code>` with
an optional `name` query parameter. Open it from a QR code or paste it into the
registration screen, verify the server, then tap Register Device. Opening a link
never enrolls automatically.

Remote images and link previews are off by default. Enable them in Privacy or
load an individual image explicitly. Image downloads have a size/time limit and
are downsampled before display; notification delivery never waits for images.
macOS also offers Launch Nod at login in Settings.

## Notification Sounds

Notification sounds are a client preference, not a request field. Change the sound in the Apple client's Subscriptions sheet. The setting is synced to the selected server as a device preference because APNs requires the provider to include the sound filename in the per-device push payload.

Bundled options:

- Default
- Ping
- Chime
- Low
- Silent

iOS does not expose the built-in Messages/Text Tone sound catalog to third-party apps. Custom sounds need to be shipped in the app bundle or present in the app container's `Library/Sounds` directory, and must be shorter than 30 seconds.
