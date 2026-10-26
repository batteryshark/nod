import Combine
import Foundation
import XCTest

@testable import NodKit

private actor ResumeFixture {
  var refreshes = 0
  var connects = 0
  var counts = ["a": 2, "b": 3]
  var informational = false
  var submissions = 0
  var status = "pending"
  var selected = "info"
  var actionable = false
  var failSubmit = false
  var otherRow = false
  var valueRequest = false
  var refreshError: String?
  var snapshotError: String?
  var actionError: String?

  func failRefresh(_ message: String?) { refreshError = message; snapshotError = message }
  func failActions(_ message: String) { actionError = message }

  func setCounts(_ counts: [String: Int]) { self.counts = counts }
  func calls() -> (Int, Int) { (refreshes, connects) }
  func includeInformationalRequest() { informational = true }
  func submissionCount() -> Int { submissions }
  func resolveElsewhere() { status = "resolved" }
  func setActionable() { actionable = true }
  func failSubmissions() { failSubmit = true }
  func includeOtherRow() { otherRow = true }
  func useValueRequest() { valueRequest = true }

  func request(_ input: String) async -> String {
    let rpc = try! JSONSerialization.jsonObject(with: Data(input.utf8)) as! [String: Any]
    let method = rpc["method"] as! String
    let params = rpc["params"] as? [String: Any] ?? [:]
    if method == "refresh" {
      refreshes += 1
      try? await Task.sleep(for: .milliseconds(80))
      if let refreshError {
        return String(decoding: try! JSONSerialization.data(withJSONObject: ["ok": false, "error": refreshError]), as: UTF8.self)
      }
    }
    if ["submit_request_option", "set_subscription", "rename_device", "enroll"].contains(method), let actionError {
      snapshotError = actionError
      return String(decoding: try! JSONSerialization.data(withJSONObject: ["ok": false, "error": actionError]), as: UTF8.self)
    }
    if method == "connect_sync" { connects += 1 }
    if method == "select_request" || method == "open_request" { selected = params["request_id"] as? String ?? selected }
    if method == "submit_request_option" {
      submissions += 1
      if failSubmit { return "{\"ok\":false,\"error\":\"transport failed\"}" }
      status = "resolved"
    }
    var state: [String: Any] = [
      "servers": [["id": "fixture", "name": "Fixture", "base_url_string": "https://fixture.invalid", "device_name": "Fixture"]],
      "selected_server_id": "fixture", "is_registered": true,
      "is_sync_connected": true, "sync_phase": "current",
      "pending_counts_by_channel": counts,
      "last_synced_at": ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: 1_790_000_000 + Double(refreshes))),
    ]
    if let snapshotError { state["last_error"] = snapshotError }
    if informational {
      state["selected_request_id"] = selected
      state["pending_counts_by_channel"] = ["a": status == "pending" ? 1 : 0]
      var row: [String: Any] = [
        "id": "info", "request_id": "info", "channel_id": "a", "recipients": [], "decision_resolution": "shared",
        "title": "Fixture", "summary": "", "body_markdown": "", "fields": [], "links": [], "notification": ["redact": false],
        "status": status, "created_at": "2026-10-21T00:00:00Z", "updated_at": "2026-10-21T00:00:00Z", "options": [], "decisions": [],
        "signing": ["version": "nod-request-v1", "recipients_commitment": "fixture", "request_digest": "rendered-digest"],
      ]
      if actionable {
        row["options"] = try! JSONSerialization.jsonObject(with: JSONEncoder().encode([NodRequestOption(id: "approve", label: "Approve", kind: .custom)]))
      }
      if valueRequest {
        row["answer_field"] = [
          "kind": "choice", "label": "Pick",
          "choices": [["id": "a", "label": "A"], ["id": "b", "label": "B"]],
          "default": ["kind": "choice", "choice_id": "a"],
        ]
        row["signing"] = ["version": "nod-request-v3", "recipients_commitment": "synthetic", "request_digest": "digest"]
      }
      state["requests"] = [row]
      if otherRow {
        var rows = state["requests"] as! [[String: Any]]
        var other = rows[0]
        other["id"] = "other"
        other["request_id"] = "other"
        rows.append(other)
        state["requests"] = rows
      }
    }
    let data = try! JSONSerialization.data(withJSONObject: ["ok": true, "result": state])
    return String(decoding: data, as: UTF8.self)
  }
}

final class NodResumeRecoveryTests: XCTestCase {
  @MainActor
  func testMacExplicitReadStaysPendingUntilExactlyOneLeave() async throws {
    let fixture = ResumeFixture()
    await fixture.includeInformationalRequest()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    let previous = store.acknowledgeOnOpen
    store.acknowledgeOnOpen = true
    defer { store.acknowledgeOnOpen = previous }
    try await Task.sleep(for: .milliseconds(200))
    XCTAssertEqual(store.selectedRequestId, "info")
    await store.refresh()
    store.selectedRequestId = "info" // programmatic selection is not reading
    let before = await fixture.submissionCount()
    XCTAssertEqual(before, 0)
    let request = try XCTUnwrap(client.state?.requests.first)
    XCTAssertEqual(request.status, .pending)
    store.setMacInboxFilter(pendingOnly: true, searchText: "")
    store.selectRequestExplicitly(request.id)
    store.selectRequestExplicitly(request.id) // overlapping tap and List binding
    await store.refresh()
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertEqual(store.macInboxRequests.map(\.id), ["info"])
    XCTAssertEqual(store.totalPendingCount, 1)
    let readingCount = await fixture.submissionCount()
    XCTAssertEqual(readingCount, 0)
    store.leaveExplicitRead()
    store.leaveExplicitRead() // overlapping user leave paths
    try await Task.sleep(for: .milliseconds(80))
    await store.refresh() // hermetic transport has no observer pump; deliver completion snapshot
    try await Task.sleep(for: .milliseconds(30))
    let after = await fixture.submissionCount()
    XCTAssertEqual(after, 1)
    XCTAssertNil(store.selectedRequestId)
    XCTAssertTrue(store.macInboxRequests.isEmpty)
    store.beginExplicitRead(request: request) // stale history value cannot resubmit
    store.leaveExplicitRead()
    try await Task.sleep(for: .milliseconds(30))
    let finalCount = await fixture.submissionCount()
    XCTAssertEqual(finalCount, 1)
  }

  @MainActor
  func testMacPendingFilterClearsHandledDetailAndSnapshotCannotRestoreIt() async throws {
    let fixture = ResumeFixture()
    await fixture.includeInformationalRequest()
    await fixture.resolveElsewhere()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await Task.sleep(for: .milliseconds(200))
    XCTAssertEqual(store.selectedRequestId, "info")
    store.setMacInboxFilter(pendingOnly: true, searchText: "")
    XCTAssertNil(store.selectedRequestId)
    XCTAssertTrue(store.macInboxRequests.isEmpty)
    await store.refresh()
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertNil(store.selectedRequestId)
    let count = await fixture.submissionCount()
    XCTAssertEqual(count, 0)
  }

  @MainActor
  func testMacUserNavigationLeavesButRepeatedRowDoesNot() async throws {
    for leave in ["filter", "channel", "history", "notification", "row"] {
      let fixture = ResumeFixture()
      await fixture.includeInformationalRequest()
      if leave == "row" { await fixture.includeOtherRow() }
      let client = NodRuntimeClient(request: { await fixture.request($0) })
      let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
        configureNotificationController: false, runtime: client)
      let previous = store.acknowledgeOnOpen
      store.acknowledgeOnOpen = true
      defer { store.acknowledgeOnOpen = previous }
      try await Task.sleep(for: .milliseconds(200))
      store.selectRequestExplicitly("info")
      store.selectRequestExplicitly("info")
      switch leave {
      case "filter": store.setMacInboxFilter(pendingOnly: true, searchText: "")
      case "channel": store.selectChannelExplicitly("b")
      case "notification":
        await store.openNotification(NodNotificationTarget(serverId: "fixture", requestId: "info", channelId: "a"))
        let stillReading = await fixture.submissionCount()
        XCTAssertEqual(stillReading, 0, "opening the same notification is not leaving")
        await store.openNotification(NodNotificationTarget(serverId: "fixture", requestId: "other", channelId: "a"))
      case "row": store.selectRequestExplicitly("other")
      default: store.leaveExplicitRead() // Search History / Done / explicit Back
      }
      try await Task.sleep(for: .milliseconds(80))
      let count = await fixture.submissionCount()
      XCTAssertEqual(count, 1, leave)
    }
  }

  @MainActor
  func testMacStartupRefreshProgrammaticSelectionAndTeardownNeverAcknowledge() async throws {
    let fixture = ResumeFixture()
    await fixture.includeInformationalRequest()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    var store: NodStore? = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await Task.sleep(for: .milliseconds(200))
    await store?.refresh()
    store?.selectedRequestId = "info"
    store?.selectRequestExplicitly("info")
    await store?.resumeFromForeground()
    store?.selectRequestExplicitly(nil) // List reconciliation is not explicit navigation
    store?.selectedRequestId = nil
    store?.selectedChannelId = "b"
    await store?.refresh()
    try await Task.sleep(for: .milliseconds(50))
    store?.selectedChannelId = nil
    try await Task.sleep(for: .milliseconds(30))
    store?.selectRequestExplicitly("info") // teardown with an armed explicit read
    weak var releasedStore = store
    store = nil // no onDisappear, hide or teardown ACK hook exists
    try await Task.sleep(for: .milliseconds(50))
    XCTAssertNil(releasedStore)
    let count = await fixture.submissionCount()
    XCTAssertEqual(count, 0)
  }

  @MainActor
  func testMacResolvedElsewhereActionableAndDisabledPreferenceDoNotAutoSubmit() async throws {
    for mode in ["resolved", "actionable", "disabled"] {
      let fixture = ResumeFixture()
      await fixture.includeInformationalRequest()
      if mode == "actionable" { await fixture.setActionable() }
      let client = NodRuntimeClient(request: { await fixture.request($0) })
      let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
        configureNotificationController: false, runtime: client)
      let previous = store.acknowledgeOnOpen
      store.acknowledgeOnOpen = mode != "disabled"
      defer { store.acknowledgeOnOpen = previous }
      try await Task.sleep(for: .milliseconds(200))
      store.selectRequestExplicitly("info")
      if mode == "resolved" {
        await fixture.resolveElsewhere()
        await store.refresh()
      }
      store.leaveExplicitRead()
      try await Task.sleep(for: .milliseconds(50))
      let count = await fixture.submissionCount()
      XCTAssertEqual(count, 0, mode)
    }
  }

  @MainActor
  func testMacFailedLeaveWriteIsNotBlindlyRetried() async throws {
    let fixture = ResumeFixture()
    await fixture.includeInformationalRequest()
    await fixture.failSubmissions()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    let previous = store.acknowledgeOnOpen
    store.acknowledgeOnOpen = true
    defer { store.acknowledgeOnOpen = previous }
    try await Task.sleep(for: .milliseconds(200))
    store.selectRequestExplicitly("info")
    store.leaveExplicitRead()
    try await Task.sleep(for: .milliseconds(50))
    store.selectRequestExplicitly("info")
    store.leaveExplicitRead()
    try await Task.sleep(for: .milliseconds(50))
    let count = await fixture.submissionCount()
    XCTAssertEqual(count, 1)
  }

  @MainActor
  func testRuntimeCoalescesOverlappingRefreshes() async throws {
    let fixture = ResumeFixture()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    async let a: Void = client.refresh()
    async let b: Void = client.refresh()
    async let c: Void = client.refresh()
    _ = try await (a, b, c)
    let calls = await fixture.calls()
    XCTAssertEqual(calls.0, 1)
  }

  @MainActor
  func testResumeCoalescesAndKeepsHealthySyncAndBadgeCounts() async throws {
    let fixture = ResumeFixture()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await Task.sleep(for: .milliseconds(200)) // settle the normal startup task
    XCTAssertEqual(store.totalPendingCount, 5)
    var observed: [Int] = []
    let subscription = store.$pendingCountsByChannel.sink { observed.append($0.values.reduce(0, +)) }
    let before = await fixture.calls()
    await fixture.setCounts(["a": 3])
    async let a: Void = store.resumeFromForeground()
    async let b: Void = store.resumeFromForeground()
    async let c = store.refresh()
    _ = await (a, b, c)
    try await Task.sleep(for: .milliseconds(30))
    let after = await fixture.calls()
    XCTAssertEqual(after.0 - before.0, 1)
    XCTAssertEqual(after.1, before.1, "healthy socket must not be reconnected")
    XCTAssertEqual(store.totalPendingCount, 3)
    await fixture.setCounts([:])
    await store.refresh()
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertEqual(store.totalPendingCount, 0)
    XCTAssertEqual(observed, [5, 3, 0], "badge subscriber gets initial, resumed and zero counts")
    withExtendedLifetime(subscription) {}
  }

  @MainActor
  func testOnlyProvenRecoveredRefreshErrorClears() async throws {
    let fixture = ResumeFixture()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await Task.sleep(for: .milliseconds(200))
    let synced = try XCTUnwrap(client.state?.lastSyncedAt)
    store.recordRecoveryError("Nod GET /api/v1/channels failed with 502")
    store.clearRecoveredError(serverId: "fixture", syncedAt: synced)
    XCTAssertNotNil(store.serverConnectionIssuesById["fixture"], "old snapshot is not proof of recovery")
    XCTAssertNil(store.alertMessage)
    store.clearRecoveredError(serverId: "other", syncedAt: synced.addingTimeInterval(1))
    XCTAssertNotNil(store.serverConnectionIssuesById["fixture"])
    store.clearRecoveredError(serverId: "fixture", syncedAt: synced.addingTimeInterval(1))
    XCTAssertNil(store.serverConnectionIssuesById["fixture"])
    store.recordRecoveryError("refresh failed")
    await store.refresh()
    XCTAssertNil(store.serverConnectionIssuesById["fixture"], "a later successful same-server snapshot clears refresh failure")
    store.recordRecoveryError("refresh failed")
    store.mapRuntimeError(NodRuntimeError.rpc("decision signing failed"))
    await store.refresh()
    XCTAssertEqual(store.lastError, "decision signing failed")
    store.recordRecoveryError("background sync failed")
    XCTAssertEqual(store.lastError, "decision signing failed")
    store.lastError = nil
    store.recordRecoveryError("same message")
    store.mapRuntimeError(NodRuntimeError.rpc("same message"))
    store.clearRecoveredError(serverId: "fixture", syncedAt: synced.addingTimeInterval(100))
    XCTAssertEqual(store.lastError, "same message", "operation ownership matters, not text alone")
  }

  // MARK: - Acknowledge on a pending informational card (both platforms)

  @MainActor
  func testBackgroundFailuresStayInStatusThroughPollingAndClearOnRecovery() async throws {
    for message in ["Nod GET /api/v1/channels failed with 500 Internal Server Error (3 attempt(s)): server returned an unsuccessful response", "network connection lost", "request timed out"] {
      let fixture = ResumeFixture()
      let client = NodRuntimeClient(request: { await fixture.request($0) })
      let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
        configureNotificationController: false, runtime: client)
      try await Task.sleep(for: .milliseconds(200))
      await fixture.failRefresh(message)
      await store.resumeFromForeground()
      let failed = await store.refresh()
      XCTAssertFalse(failed, "iOS background fetch must report the refresh outcome, not alert ownership")
      try await client.selectRequest("info") // poll the runtime's echoed last_error
      try await Task.sleep(for: .milliseconds(30))
      XCTAssertEqual(store.connectionIssue(for: try XCTUnwrap(store.selectedServer)), message)
      XCTAssertNil(store.alertMessage)
      await fixture.failRefresh(nil)
      let recovered = await store.refresh()
      XCTAssertTrue(recovered)
      try await Task.sleep(for: .milliseconds(30))
      XCTAssertNil(store.serverConnectionIssuesById["fixture"])
      XCTAssertNil(store.alertMessage)
    }
  }

  @MainActor
  func testExplicitFailuresSurviveBackgroundRecoveryAndDismissedEchoStaysQuiet() async throws {
    for message in ["Nod POST /api/v1/requests/info/decisions failed with 500 Internal Server Error (1 attempt(s)): server returned an unsuccessful response", "network connection lost", "decision signing failed", "Request changed. Review it again before answering."] {
      let (store, client, fixture) = try await informationalFixtureStore()
      await fixture.failActions(message)
      let result = await store.dismissIfInformational(request: try XCTUnwrap(client.state?.requests.first))
      XCTAssertFalse(result)
      XCTAssertEqual(store.alertMessage, message)
      try await client.selectRequest("info") // action failure echoed by polling
      try await Task.sleep(for: .milliseconds(30))
      store.recordRecoveryError("background timeout")
      await fixture.failRefresh(nil)
      await store.refresh()
      try await Task.sleep(for: .milliseconds(30))
      XCTAssertEqual(store.alertMessage, message)
      await store.setSubscription(channelId: "a", subscribed: true)
      XCTAssertEqual(store.alertMessage, message)
      store.dismissAlertMessage()
      try await client.selectRequest("info")
      try await Task.sleep(for: .milliseconds(30))
      XCTAssertNil(store.alertMessage, "polling an action failure must not resurrect its modal")
    }
  }

  @MainActor
  func testBackgroundAuthRequiresReEnrollmentAndPermissionAlertSurvivesRecovery() async throws {
    let (store, _, fixture) = try await informationalFixtureStore()
    store.notificationPermissionIssue = "Notifications are disabled."
    store.recordRecoveryError("websocket connection lost")
    XCTAssertEqual(store.alertMessage, "Notifications are disabled.")
    await store.refresh()
    XCTAssertEqual(store.alertMessage, "Notifications are disabled.")
    await fixture.failRefresh("Nod GET /api/v1/channels failed with 401 Unauthorized (1 attempt(s)): server returned an unsuccessful response")
    await store.refresh()
    XCTAssertTrue(store.canReEnrollInvalidSession)
    XCTAssertTrue(store.alertMessage?.contains("Re-enroll") == true)
    await fixture.failRefresh(nil)
    await store.refresh()
    XCTAssertTrue(store.canReEnrollInvalidSession)
    XCTAssertTrue(store.alertMessage?.contains("Re-enroll") == true)
    XCTAssertNotNil(store.serverConnectionIssuesById["fixture"])
  }

  @MainActor
  func testWebsocketEventsUseStatusExceptAuthAndDecodeFailures() async throws {
    let (store, client, _) = try await informationalFixtureStore()
    client.apply(.transientError(message: "websocket network timeout"))
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertNil(store.alertMessage)
    XCTAssertEqual(store.serverConnectionIssuesById["fixture"], "websocket network timeout")
    await store.refresh()
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertNil(store.serverConnectionIssuesById["fixture"])
    client.apply(.transientError(message: "runtime event decode failed: fixture"))
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertEqual(store.alertMessage, "runtime event decode failed: fixture")
    client.apply(.authRevoked)
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertTrue(store.canReEnrollInvalidSession)
    XCTAssertTrue(store.alertMessage?.contains("Re-enroll") == true)
  }

  @MainActor
  func testEnrollmentAuthRejectionIsNotSelectedSessionRevocation() async throws {
    let (store, client, fixture) = try await informationalFixtureStore()
    let message = "Nod POST /api/v1/enroll failed with 401 Unauthorized (1 attempt(s)): server returned an unsuccessful response"
    // Enrollment's catch uses this action mapping (without provisioning real keys).
    store.mapRuntimeError(NodRuntimeError.rpc(message), checkAuth: false)
    await fixture.failActions(message)
    try? await client.enroll(baseURL: "https://fixture.invalid", deviceName: "Fixture", code: "INVALID", notificationSound: nil, platform: .macos)
    try await client.selectRequest("info")
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertEqual(store.alertMessage, message)
    XCTAssertFalse(store.canReEnrollInvalidSession)
  }

  @MainActor
  func testRuntimeInitiatedResyncAuthFailureStillOffersReEnrollment() async throws {
    let (store, client, fixture) = try await informationalFixtureStore()
    await fixture.failRefresh("Nod GET /api/v1/channels failed with 401 Unauthorized (1 attempt(s)): server returned an unsuccessful response")
    client.apply(.resyncRequired)
    try await Task.sleep(for: .milliseconds(150))
    XCTAssertTrue(store.canReEnrollInvalidSession)
    XCTAssertTrue(store.alertMessage?.contains("Re-enroll") == true)
  }

  /// The detail view is SwiftUI in the app targets, so it cannot be hosted by
  /// this package's tests. What it branches on is `NodRequest.isPendingInformational`,
  /// the shared rule this covers: the button must be offered with the
  /// auto-acknowledge preference on and off, and never for a card that has
  /// options, is resolved, or asks for a value.
  @MainActor
  func testAcknowledgeIsOfferedForPendingInformationalCardsWithEitherPreference() async throws {
    let defaultsKey = "nod.acknowledgeOnOpen"
    let previous = UserDefaults.standard.object(forKey: defaultsKey) as? Bool
    defer {
      if let previous { UserDefaults.standard.set(previous, forKey: defaultsKey) }
      else { UserDefaults.standard.removeObject(forKey: defaultsKey) }
    }

    for preference in [true, false] {
      let (store, client, _) = try await informationalFixtureStore()
      store.acknowledgeOnOpen = preference
      await store.refresh()
      let request = try XCTUnwrap(client.state?.requests.first)
      XCTAssertEqual(store.acknowledgeOnOpen, preference)
      XCTAssertTrue(
        request.isPendingInformational,
        "Acknowledge must be offered with acknowledgeOnOpen = \(preference)")
    }

    for exclusion in ["actionable", "resolved", "value"] {
      let fixture = ResumeFixture()
      await fixture.includeInformationalRequest()
      switch exclusion {
      case "actionable": await fixture.setActionable()
      case "resolved": await fixture.resolveElsewhere()
      default: await fixture.useValueRequest()
      }
      let client = NodRuntimeClient(request: { await fixture.request($0) })
      let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
        configureNotificationController: false, runtime: client)
      try await Task.sleep(for: .milliseconds(200))
      await store.refresh()
      let request = try XCTUnwrap(client.state?.requests.first)
      XCTAssertFalse(request.isPendingInformational, "\(exclusion) cards must not offer Acknowledge")
    }
  }

  /// Tapping Acknowledge must clear the card once. The leave-on-read auto path
  /// can run right after the tap, so it must find the card resolved and stay
  /// quiet rather than submitting a second dismiss.
  @MainActor
  func testManualAcknowledgeThenLeaveSubmitsExactlyOnce() async throws {
    let (store, client, fixture) = try await informationalFixtureStore()
    let previous = store.acknowledgeOnOpen
    store.acknowledgeOnOpen = true
    defer { store.acknowledgeOnOpen = previous }
    let request = try XCTUnwrap(client.state?.requests.first)
    store.selectRequestExplicitly(request.id) // arms the auto-acknowledge path

    let submitted = await store.dismissIfInformational(request: request, serverId: "fixture")
    XCTAssertTrue(submitted)
    let afterTap = await fixture.submissionCount()
    XCTAssertEqual(afterTap, 1)

    await store.refresh() // resolved card reaches the snapshot, so the button disappears
    XCTAssertEqual(client.state?.requests.first?.status, .resolved)
    store.leaveExplicitRead() // the auto path sees nothing pending
    try await Task.sleep(for: .milliseconds(80))
    await store.refresh()
    let afterLeave = await fixture.submissionCount()
    XCTAssertEqual(afterLeave, 1, "the auto path must not submit a second acknowledge")
    XCTAssertEqual(store.totalPendingCount, 0)
    XCTAssertFalse(store.isSubmitting(request.id))
  }

  /// Two taps before the first write settles are one write: the per-request
  /// `pendingActions` guard admits the first and refuses the second.
  @MainActor
  func testOverlappingAcknowledgeTapsSubmitOnce() async throws {
    let (store, client, fixture) = try await informationalFixtureStore()
    let previous = store.acknowledgeOnOpen
    store.acknowledgeOnOpen = true
    defer { store.acknowledgeOnOpen = previous }
    let request = try XCTUnwrap(client.state?.requests.first)

    async let first = store.dismissIfInformational(request: request, serverId: "fixture")
    async let second = store.dismissIfInformational(request: request, serverId: "fixture")
    let results = [await first, await second]
    XCTAssertEqual(results.filter { $0 }.count, 1, "the guard admits exactly one submit")
    let count = await fixture.submissionCount()
    XCTAssertEqual(count, 1)
  }

  @MainActor
  private func informationalFixtureStore() async throws -> (NodStore, NodRuntimeClient, ResumeFixture) {
    let fixture = ResumeFixture()
    await fixture.includeInformationalRequest()
    let client = NodRuntimeClient(request: { await fixture.request($0) })
    let store = NodStore(platform: .macos, defaultDeviceName: "Fixture", presentLocalNotifications: false,
      configureNotificationController: false, runtime: client)
    try await Task.sleep(for: .milliseconds(200))
    await store.refresh()
    return (store, client, fixture)
  }
}
