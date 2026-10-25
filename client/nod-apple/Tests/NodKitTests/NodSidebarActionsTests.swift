import Combine
import Foundation
import XCTest
@testable import NodKit

private func sidebarRequest(_ changes: [String: Any] = [:]) throws -> NodRequest {
  var row: [String: Any] = [
    "id": "card", "request_id": "card", "channel_id": "channel", "title": "Card",
    "summary": "", "body_markdown": "", "notification": ["redact": false],
    "status": "pending", "created_at": "2026-10-21T00:00:00Z", "updated_at": "2026-10-21T00:00:00Z",
    "options": [], "signing": ["version": "nod-request-v1", "recipients_commitment": "fixture", "request_digest": "rendered-digest"],
  ]
  row.merge(changes) { _, new in new }
  row["options"] = (row["options"] as! [[String: Any]]).map { option in
    ["style": "default", "requires_text": false, "destructive": false, "foreground": false]
      .merging(option) { _, new in new }
  }
  return try JSONDecoder.nod.decode(NodRequest.self, from: JSONSerialization.data(withJSONObject: row))
}

private actor SidebarFixture {
  var row: NodRequest
  var server = "original"
  var writes: [[String: String]] = []
  var fail = false
  var hold = false
  var entered: CheckedContinuation<Void, Never>?
  var release: CheckedContinuation<Void, Never>?
  init(_ row: NodRequest) { self.row = row }
  func replace(_ row: NodRequest) { self.row = row }
  func switchServer() { server = "other" }
  func failWrites() { fail = true }
  func holdWrite() { hold = true }
  func waitForWrite() async {
    if !writes.isEmpty { return }
    await withCheckedContinuation { entered = $0 }
  }
  func releaseWrite() { release?.resume(); release = nil }
  func captured() -> [[String: String]] { writes }
  func request(_ input: String) async -> String {
    let rpc = try! JSONSerialization.jsonObject(with: Data(input.utf8)) as! [String: Any]
    let method = rpc["method"] as! String
    let params = rpc["params"] as? [String: Any] ?? [:]
    if method == "submit_request_option" {
      writes.append(params as! [String: String])
      if hold {
        await withCheckedContinuation { continuation in
          release = continuation
          entered?.resume(); entered = nil
        }
      }
      if fail { return "{\"ok\":false,\"error\":\"transport uncertain\"}" }
    }
    var encoded = try! JSONSerialization.jsonObject(with: JSONEncoder.nod.encode(row)) as! [String: Any]
    if method == "get_request" && fail { return "{\"ok\":false,\"error\":\"status unavailable\"}" }
    if let write = writes.first, !fail {
      encoded["status"] = "resolved"
      encoded["decision"] = ["request_id": "card", "option_id": write["option_id"]!, "option_kind": "custom", "option_label": "Recorded", "resolved_at": "2026-10-21T00:00:00Z"]
    }
    let state: [String: Any] = [
      "servers": ["original", "other"].map { ["id": $0, "name": $0, "base_url_string": "https://fixture.invalid", "device_name": "Fixture"] },
      "selected_server_id": server, "selected_request_id": "card", "is_registered": false,
      "requests": [encoded],
    ]
    let result = method == "get_request" ? ["request": encoded] : state
    return String(decoding: try! JSONSerialization.data(withJSONObject: ["ok": true, "result": result]), as: UTF8.self)
  }
}

@MainActor
final class NodSidebarActionsTests: XCTestCase {
  private func setup(_ row: NodRequest) async throws -> (NodStore, SidebarFixture) {
    let fixture = SidebarFixture(row)
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await store.ensureRuntimeStarted()
    let ready = expectation(description: "Runtime state mirrored to store")
    let subscription = store.$selectedServerId.filter { $0 == "original" }.first().sink { _ in ready.fulfill() }
    await fulfillment(of: [ready], timeout: 5)
    withExtendedLifetime(subscription) {}
    return (store, fixture)
  }

  func testInformationalMenuIsIndependentOfPreferenceAndDoesNotNavigate() async throws {
    let row = try sidebarRequest()
    let (store, fixture) = try await setup(row)
    let previous = store.acknowledgeOnOpen
    defer { store.acknowledgeOnOpen = previous }
    for preference in [false, true] {
      store.acknowledgeOnOpen = preference
      let selection = store.selectedRequestId
      let actions = store.sidebarActions(for: row, serverId: "original")
      XCTAssertEqual(actions.map(\.label), ["Acknowledge"])
      XCTAssertTrue(store.canPerformSidebarAction(actions[0], request: row, serverId: "original"))
      XCTAssertEqual(store.selectedRequestId, selection)
    }
    let writes = await fixture.captured()
    XCTAssertTrue(writes.isEmpty)
  }

  func testMixedOptionsAndValueCards() throws {
    let valueField = NodAnswerField(kind: .choice, label: "Pick",
      choices: [NodAnswerChoice(id: "a", label: "A"), NodAnswerChoice(id: "b", label: "B")], defaultValue: .choice("a"))
    let options: [[String: Any]] = [
      ["id": "yes", "label": "Yes", "kind": "approve", "text_placeholder": "Optional notes"],
      ["id": "custom", "label": "Custom", "kind": "custom"],
      ["id": "no", "label": "No", "kind": "reject"],
      ["id": "danger", "label": "Danger", "kind": "reject", "destructive": true],
      ["id": "style", "label": "Style", "kind": "custom", "style": "destructive"],
      ["id": "text", "label": "Text", "kind": "custom", "requires_text": true],
      ["id": "at", "label": "AT", "kind": "approve_with_text", "requires_text": false],
      ["id": "rt", "label": "RT", "kind": "reject_with_text", "requires_text": false],
      ["id": "open", "label": "Open", "kind": "open"],
    ]
    XCTAssertEqual(NodSidebarActions.options(for: try sidebarRequest(["options": options]), userId: nil).map(\.id), ["yes", "custom", "no"])
    for changes: [String: Any] in [
      ["options": [["id": "__nod_value_open", "label": "Open", "kind": "open"]]],
      ["signing": ["version": "nod-request-v3", "recipients_commitment": "fixture", "request_digest": "digest"]],
      ["answer_field": try JSONSerialization.jsonObject(with: JSONEncoder.nod.encode(valueField))],
    ] {
      XCTAssertTrue(NodSidebarActions.options(for: try sidebarRequest(changes), userId: nil).isEmpty)
    }
  }

  func testTerminalExpiredAndOwnPerUserAnswer() throws {
    for status in ["resolved", "cancelled", "expired"] {
      XCTAssertTrue(NodSidebarActions.options(for: try sidebarRequest(["status": status]), userId: nil).isEmpty)
    }
    XCTAssertTrue(NodSidebarActions.options(for: try sidebarRequest(["expires_at": "2020-01-01T00:00:00Z"]), userId: nil).isEmpty)
    let row = try sidebarRequest(["decision_resolution": "per_user", "decisions": [
      ["user_id": "me", "decision": ["request_id": "card", "option_id": "dismiss", "option_kind": "dismiss", "option_label": "Acknowledge", "resolved_at": "2026-10-21T00:00:00Z"]],
    ]])
    XCTAssertTrue(NodSidebarActions.options(for: row, userId: "me").isEmpty)
    XCTAssertTrue(NodSidebarActions.options(for: row, userId: nil).isEmpty)
    XCTAssertEqual(NodSidebarActions.options(for: row, userId: "someone-else").map(\.id), ["dismiss"])
  }

  func testClickRevalidatesDigestActionAttemptsAndServer() async throws {
    let row = try sidebarRequest()
    let (store, fixture) = try await setup(row)
    let action = try XCTUnwrap(store.sidebarActions(for: row, serverId: "original").first)
    let unknown = NodRequestOption(id: "unknown", label: "Unknown", kind: .custom)
    XCTAssertFalse(store.canPerformSidebarAction(unknown, request: row, serverId: "original"))
    let unsigned = try sidebarRequest(["signing": NSNull()])
    XCTAssertFalse(store.canPerformSidebarAction(action, request: unsigned, serverId: "original"))
    for submitting in [true, false] {
      store.answerAttempts["original:card"] = NodAnswerAttempt(serverId: "original", requestId: "card", optionId: "dismiss", optionLabel: "Acknowledge", isSubmitting: submitting)
      let accepted = await store.performSidebarAction(action, request: row, serverId: "original")
      XCTAssertFalse(accepted)
    }
    store.answerAttempts = [:]
    for replacement in [try sidebarRequest(["status": "resolved"]), try sidebarRequest(["signing": ["version": "nod-request-v1", "recipients_commitment": "fixture", "request_digest": "unseen"]])] {
      await fixture.replace(replacement)
      try await store.runtime.refresh()
      XCTAssertFalse(store.canPerformSidebarAction(action, request: row, serverId: "original"))
    }
    await fixture.replace(row)
    await fixture.switchServer()
    try await store.runtime.refresh()
    let accepted = await store.performSidebarAction(action, request: row, serverId: "original")
    XCTAssertFalse(accepted)
    let writes = await fixture.captured()
    XCTAssertTrue(writes.isEmpty)
  }

  func testSignedClickOverlapAndUncertainRetryAreGuarded() async throws {
    for info in [true, false] {
      let row = try sidebarRequest(info ? [:] : ["options": [["id": "yes", "label": "Yes", "kind": "approve"]]])
      let (store, fixture) = try await setup(row)
      let action = try XCTUnwrap(store.sidebarActions(for: row, serverId: "original").first)
      await fixture.holdWrite()
      await fixture.failWrites()
      let first = Task { await store.performSidebarAction(action, request: row, serverId: "original") }
      await fixture.waitForWrite()
      let overlap = await store.performSidebarAction(action, request: row, serverId: "original")
      XCTAssertFalse(overlap)
      if info {
        let auto = await store.dismissIfInformational(request: row, serverId: "original")
        XCTAssertFalse(auto)
      }
      await fixture.releaseWrite()
      _ = await first.value
      XCTAssertNotNil(store.answerAttempt(requestId: row.id, serverId: "original"))
      let retry = await store.performSidebarAction(action, request: row, serverId: "original")
      XCTAssertFalse(retry)
      if info {
        let autoRetry = await store.dismissIfInformational(request: row, serverId: "original")
        XCTAssertFalse(autoRetry)
      }
      let writes = await fixture.captured()
      XCTAssertEqual(writes, [["server_id": "original", "request_id": "card", "option_id": info ? "dismiss" : "yes", "expected_request_digest": "rendered-digest"]])
    }
  }

  func testSuccessfulClickUsesRenderedSignedPathAndGETReceipt() async throws {
    for info in [true, false] {
      let row = try sidebarRequest(info ? [:] : ["options": [["id": "yes", "label": "Yes", "kind": "approve"]]])
      let (store, fixture) = try await setup(row)
      let option = try XCTUnwrap(store.sidebarActions(for: row, serverId: "original").first)
      let result = await store.performSidebarAction(option, request: row, serverId: "original")
      XCTAssertTrue(result)
      XCTAssertEqual(store.answerReceipt?.request.status, .resolved)
      XCTAssertNil(store.answerAttempt(requestId: "card", serverId: "original"))
      let repeated = await store.performSidebarAction(option, request: row, serverId: "original")
      XCTAssertFalse(repeated)
      let writes = await fixture.captured()
      XCTAssertEqual(writes, [["server_id": "original", "request_id": "card", "option_id": info ? "dismiss" : "yes", "expected_request_digest": "rendered-digest"]])
    }
  }

  func testAutoAcknowledgementAlreadyInflightDeniesMenuClick() async throws {
    let row = try sidebarRequest()
    let (store, fixture) = try await setup(row)
    let action = try XCTUnwrap(store.sidebarActions(for: row, serverId: "original").first)
    await fixture.holdWrite()
    let auto = Task { await store.dismissIfInformational(request: row, serverId: "original") }
    await fixture.waitForWrite()
    XCTAssertFalse(store.canPerformSidebarAction(action, request: row, serverId: "original"))
    let clicked = await store.performSidebarAction(action, request: row, serverId: "original")
    XCTAssertFalse(clicked)
    await fixture.releaseWrite()
    let acknowledged = await auto.value
    XCTAssertTrue(acknowledged)
    let writes = await fixture.captured()
    XCTAssertEqual(writes.count, 1)
  }
}
