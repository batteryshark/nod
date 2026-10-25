import Foundation
import XCTest

import NodClientFFI

@testable import NodKit

/// Drives the real shared Rust runtime through `NodRuntimeClient` and asserts
/// the `NodClientMessage` envelopes decode into `NodRuntimeState` — i.e. NodKit
/// can consume nod-client-core's view state directly, the prerequisite for
/// deleting the duplicated Swift client.
final class NodRuntimeClientTests: XCTestCase {
  private final class StubSigner: NodDeviceSigner, @unchecked Sendable {
    func provision(profileId: String) throws -> NodDeviceKey {
      throw SignerCallbackError.Failed(message: "unused")
    }
    func signingKey(profileId: String) throws -> NodDeviceKey? { nil }
    func sign(profileId: String, payload: Data) throws -> String {
      throw SignerCallbackError.Failed(message: "unused")
    }
    func remove(profileId: String) throws {}
  }

  override func setUp() {
    super.setUp()
    let dir = NSTemporaryDirectory() + "nod-runtime-client-\(ProcessInfo.processInfo.processIdentifier)"
    try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    setenv("NOD_CLIENT_CORE_STATE_DIR", dir, 1)
    setenv("NOD_CLIENT_CORE_INSECURE_TOKEN_STORE", "1", 1)
  }

  @MainActor
  func testStartDecodesInitialStateAndRoundTripsRPC() async throws {
    let client = NodRuntimeClient(signer: StubSigner())
    try await client.start()

    // The initial `ready` + `state` envelopes arrive on the main actor.
    for _ in 0..<50 {
      if client.state != nil && client.statePath != nil { break }
      try await Task.sleep(nanoseconds: 10_000_000)
    }

    let state = try XCTUnwrap(client.state, "runtime should have emitted a decoded state")
    XCTAssertFalse(client.statePath?.isEmpty ?? true)
    // A fresh hermetic store: not registered, not connected, no servers.
    XCTAssertFalse(state.isRegistered)
    XCTAssertFalse(state.isSyncConnected)
    XCTAssertTrue(state.servers.isEmpty)
    XCTAssertEqual(state.totalPendingCount, 0)

    // An RPC that fails cleanly (no selected server) surfaces as a thrown error,
    // not a crash — proving the request/response envelope round-trips.
    do {
      try await client.refresh()
      XCTFail("refresh without a server should fail")
    } catch let NodRuntimeError.rpc(message) {
      XCTAssertFalse(message.isEmpty)
    }
  }

  /// Every submit RPC names the rendered digest; the runtime treats the field as
  /// required and refuses to sign a request whose digest differs.
  @MainActor
  func testSubmitRPCsAlwaysCarryExpectedRequestDigest() async throws {
    actor Transport {
      var bodies: [String] = []
      func record(_ body: String) -> String {
        if body.contains(#""method":"state""#) {
          return #"{"ok":true,"result":{"servers":[],"is_registered":false,"requests":[],"pending_counts_by_channel":{}}}"#
        }
        bodies.append(body)
        return #"{"ok":true,"result":{}}"#
      }
      func recorded() -> [String] { bodies }
    }
    let transport = Transport()
    let client = NodRuntimeClient(request: { await transport.record($0) })
    try await client.start()

    try await client.submitRequestOption(serverId: "s", requestId: "r", optionId: "approve", text: nil, expectedRequestDigest: "digest-of-rendered-request")
    try await client.submitOption(requestId: "r", optionId: "reject", text: "why", expectedRequestDigest: "digest-of-rendered-request")

    let calls = try await transport.recorded().map { try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]) }
    XCTAssertEqual(calls.map { $0["method"] as? String }, ["submit_request_option", "submit_option"])
    for call in calls {
      let params = try XCTUnwrap(call["params"] as? [String: Any])
      XCTAssertEqual(params["expected_request_digest"] as? String, "digest-of-rendered-request")
      XCTAssertNil(params["answer_value"])
      XCTAssertNil(params["signature"], "Signing bytes are built only by shared Rust")
    }
    XCTAssertEqual((calls[0]["params"] as? [String: Any])?["server_id"] as? String, "s")
    XCTAssertEqual((calls[1]["params"] as? [String: Any])?["text"] as? String, "why")
  }
}
