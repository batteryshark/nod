import Foundation
import XCTest
import UserNotifications
@testable import NodKit

final class NodNotificationPolicyTests: XCTestCase {
  func testPushDeviceIdentitySelectsOriginatingServer() {
    let target = NodNotificationTarget.decode(["nod": ["device_id": "device-a", "request_id": "request", "channel_id": "deploy"]])
    XCTAssertEqual(NodNotificationPolicy.serverId(for: target, servers: servers), "a")
  }

  func testLegacyPushDoesNotGuessBetweenServers() {
    let target = NodNotificationTarget(requestId: "request")
    XCTAssertNil(NodNotificationPolicy.serverId(for: target, servers: servers))
    XCTAssertEqual(NodNotificationPolicy.serverId(for: target, servers: [servers[0]]), "a")
  }

  func testUnknownExplicitServerNeverFallsBack() {
    let target = NodNotificationTarget(serverId: "removed", requestId: "request")
    XCTAssertNil(NodNotificationPolicy.serverId(for: target, servers: [servers[0]]))
  }

  func testRedactedPreviewDoesNotExposeRequestContent() throws {
    let request = try makeRequest(notification: ["redact": true])
    let preview = NodNotificationPolicy.preview(for: request)
    XCTAssertFalse(preview.title.contains("secret"))
    XCTAssertFalse(preview.body.contains("secret"))
  }

  func testExplicitSafePreviewHonorsUserHidePreference() throws {
    let request = try makeRequest(notification: ["redact": true, "title": "Safe title", "body": "Safe body"])
    let preview = NodNotificationPolicy.preview(for: request)
    XCTAssertEqual(preview.title, "Safe title")
    XCTAssertEqual(preview.body, "Safe body")
    XCTAssertEqual(NodNotificationPolicy.preview(for: request, hidePreviews: true).title, "Nod")
  }

  func testNotificationEventsCarryServerIdentity() throws {
    let data = Data(#"{"kind":"notification_removed","payload":{"server_id":"a","request_id":"request"}}"#.utf8)
    guard case .notificationRemoved(let serverId, let requestId) = try NodRuntimeMessage(from: data) else {
      return XCTFail("Expected removal")
    }
    XCTAssertEqual(serverId, "a")
    XCTAssertEqual(requestId, "request")
  }

  func testEnrollmentLinkPreservesServerPrefixAndNormalizesCode() throws {
    let link = try NodEnrollmentLink(url: XCTUnwrap(URL(string: "nod://enroll?server=https%3A%2F%2Fnod.test%2Fteam&code=abcd1234")))
    XCTAssertEqual(link.serverURL, "https://nod.test/team")
    XCTAssertEqual(link.code, "ABCD1234")
  }

  func testEnrollmentLinkRejectsAmbiguousFieldsAndCredentials() throws {
    for value in ["nod://enroll?server=https://nod.test&code=ABCDEFGH&code=12345678", "nod://enroll?server=https://user@nod.test&code=ABCDEFGH"] {
      XCTAssertThrowsError(try NodEnrollmentLink(url: XCTUnwrap(URL(string: value))))
    }
  }

  func testNativeActionsKeepCustomLabelsIdsAndForegroundRequirements() throws {
    let request = try makeRequest(notification: [:], options: [
      ["id": "deploy_staging", "label": "Deploy to staging", "kind": "approve", "style": "default", "requires_text": false, "destructive": false, "foreground": true]
    ])
    let action = try XCTUnwrap(NodNotificationController.actions(for: request).first)
    XCTAssertEqual(action.identifier, "nod.option.deploy_staging")
    XCTAssertEqual(action.title, "Deploy to staging")
    XCTAssertTrue(action.options.contains(.foreground))
    XCTAssertTrue(action.options.contains(.authenticationRequired))
  }

  func testNativeTextActionOffersOptionalInputAndOpensForeground() throws {
    let request = try makeRequest(notification: [:], options: [
      ["id": "notes", "label": "Add notes", "kind": "custom", "style": "default", "requires_text": true, "destructive": false, "foreground": false]
    ])
    let action = try XCTUnwrap(NodNotificationController.actions(for: request).first as? UNTextInputNotificationAction)
    XCTAssertEqual(action.identifier, "nod.option.notes")
    XCTAssertTrue(action.options.contains(.foreground))
  }

  func testLongOrRedactedOptionSetsOfferOnlyOpen() throws {
    let option: [String: Any] = ["id": "custom", "label": "Private label", "kind": "custom", "style": "default", "requires_text": false, "destructive": false, "foreground": false]
    let redacted = try makeRequest(notification: ["redact": true], options: [option])
    let long = try makeRequest(notification: [:], options: Array(repeating: option, count: 4))
    for request in [redacted, long] {
      XCTAssertEqual(NodNotificationController.actions(for: request).map(\.identifier), ["open"])
    }
  }

  func testDeviceNotificationPreferencesDecodeMutedChannelsAndPause() throws {
    let data = Data(#"{"hide_content":true,"muted_channels":["deploys"],"snoozed_until":"2026-09-22T19:30:00.000Z"}"#.utf8)
    let preferences = try JSONDecoder.nod.decode(NodDeviceNotificationPreferences.self, from: data)
    XCTAssertTrue(preferences.hideContent)
    XCTAssertEqual(preferences.mutedChannels, ["deploys"])
    XCTAssertNotNil(preferences.snoozedUntil)
    let encoded = try JSONSerialization.jsonObject(with: JSONEncoder.nod.encode(preferences)) as? [String: Any]
    XCTAssertEqual(encoded?["hide_content"] as? Bool, true)
    XCTAssertEqual(encoded?["muted_channels"] as? [String], ["deploys"])
  }

  func testOlderNotificationPreferencesDefaultToUnmuted() throws {
    let preferences = try JSONDecoder.nod.decode(NodDeviceNotificationPreferences.self, from: Data("{}".utf8))
    XCTAssertEqual(preferences, NodDeviceNotificationPreferences())
  }

  func testChannelNotificationRemovalStaysWithinDeviceAndChannel() {
    let target = NodNotificationTarget(serverId: "a", deviceId: "device-a", channelId: "deploys")
    XCTAssertTrue(target.matches(NodNotificationTarget(deviceId: "device-a", requestId: "request", channelId: "deploys")))
    XCTAssertFalse(target.matches(NodNotificationTarget(deviceId: "device-a", requestId: "request", channelId: "billing")))
    XCTAssertFalse(target.matches(NodNotificationTarget(serverId: "b", deviceId: "device-b", requestId: "request", channelId: "deploys")))
  }

  private var servers: [NodServerProfile] {
    [NodServerProfile(id: "a", name: "A", baseURLString: "https://a.test", deviceName: "Phone", deviceId: "device-a"),
     NodServerProfile(id: "b", name: "B", baseURLString: "https://b.test", deviceName: "Phone", deviceId: "device-b")]
  }

  private func makeRequest(notification: [String: Any], options: [[String: Any]] = []) throws -> NodRequest {
    let json: [String: Any] = [
      "id": "request", "request_id": "request", "channel_id": "deploy",
      "title": "secret title", "summary": "secret summary", "body_markdown": "secret body",
      "notification": notification, "status": "pending", "options": options,
      "created_at": "2026-09-22T12:00:00Z", "updated_at": "2026-09-22T12:00:00Z"
    ]
    return try JSONDecoder.nod.decode(NodRequest.self, from: JSONSerialization.data(withJSONObject: json))
  }
}
