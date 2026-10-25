import Foundation
import XCTest
@testable import NodKit

private actor AnswerFixture {
  var writes = 0
  var reads = 0
  var failRead = true
  var resolved = false
  var perUser = false
  var cancelled = false
  var informational = false
  var navigationReads = 0
  var fieldJSON: String?
  var submitted: Data?
  var otherPending = false
  var matchingNext = false
  var selectedRequest: String?
  var missingUser = false
  var unsigned = false
  func omitUser() { missingUser = true }
  func omitSigning() { unsigned = true }
  func includeOtherPending(matchingNext: Bool = false) { otherPending = true; self.matchingNext = matchingNext }
  func valueField(_ json: String) { fieldJSON = json }
  func submittedJSON() -> Data? { submitted }
  func cancel() { cancelled = true }
  func information() { informational = true }
  func mode(readFails: Bool, resolved: Bool, perUser: Bool = false) {
    failRead = readFails; self.resolved = resolved; self.perUser = perUser
  }
  func counts() -> (Int, Int, Int) { (writes, reads, navigationReads) }
  func request(_ input: String) -> String {
    let rpc = try! JSONSerialization.jsonObject(with: Data(input.utf8)) as! [String: Any]
    let method = rpc["method"] as! String
    if method == "select_request" { selectedRequest = (rpc["params"] as? [String:Any])?["request_id"] as? String }
    if method == "open_request" { navigationReads += 1 }
    if method == "submit_request_option" {
      writes += 1; submitted = try! JSONSerialization.data(withJSONObject: rpc["params"] ?? [:])
      return "{\"ok\":false,\"error\":\"transport unknown\"}"
    }
    if method == "get_request" {
      reads += 1
      if failRead { return "{\"ok\":false,\"error\":\"offline\"}" }
    }
    let decision: [String: Any] = ["request_id":"card", "option_id":"yes", "option_kind":"custom", "option_label":"Yes", "actor_user_id":"me", "resolved_at":"2026-10-24T00:00:00Z"]
    var request: [String: Any] = ["id":"card", "request_id":"card", "channel_id":"channel", "recipients":[], "decision_resolution":perUser ? "per_user" : "shared", "title":"Synthetic", "summary":"", "body_markdown":"", "fields":[], "links":[], "notification":["redact":false], "status":resolved && !perUser ? "resolved" : "pending", "created_at":"2026-10-24T00:00:00Z", "updated_at":"2026-10-24T00:00:00Z", "options":[["id":"yes", "label":"Yes"]], "decisions":[]]
    if cancelled { request["status"] = "cancelled" }
    request["options"] = try! JSONSerialization.jsonObject(with: JSONEncoder().encode([NodRequestOption(id:"yes", label:"Yes", kind:.custom)]))
    if informational { request["options"] = [] }
    // Every server view carries a signing context: v2 for ordinary requests, v3 for value requests.
    request["signing"] = ["version":"nod-request-v2", "recipients_commitment":"synthetic", "request_digest":"rendered-digest"]
    if let fieldJSON {
      request["answer_field"] = try! JSONSerialization.jsonObject(with: Data(fieldJSON.utf8))
      request["signing"] = ["version":"nod-request-v3", "recipients_commitment":"synthetic", "request_digest":"rendered-digest"]
    }
    if unsigned { request.removeValue(forKey: "signing") }
    if resolved {
      if perUser { request["decisions"] = [["user_id":"me", "decision":decision]] }
      else { request["decision"] = decision }
    }
    if method == "get_request" {
      if otherPending { request["title"] = "Needle decision" }
      return String(decoding:try! JSONSerialization.data(withJSONObject:["ok":true,"result":["request":request]]),as:UTF8.self)
    }
    var rows = [request]
    if otherPending {
      rows[0]["title"] = "Needle decision"
      var other = request
      other["id"] = "other"; other["request_id"] = "other"; other["title"] = "Unrelated decision"
      other["status"] = "pending"; other["created_at"] = "2026-10-26T00:00:00Z"
      other.removeValue(forKey:"decision"); other["decisions"] = []
      rows.insert(other,at:0)
      if matchingNext {
        other["id"] = "next"; other["request_id"] = "next"; other["title"] = "Needle follow-up"
        other["created_at"] = "2026-10-23T00:00:00Z"
        rows.append(other)
      }
    }
    var state: [String: Any] = ["servers":[["id":"fixture", "name":"Synthetic", "base_url_string":"https://fixture.invalid", "device_name":"Synthetic"]], "selected_server_id":"fixture", "is_registered":true, "current_user":["id":"me", "name":"Synthetic", "created_at":"2026-10-24T00:00:00Z", "updated_at":"2026-10-24T00:00:00Z"], "requests":rows, "pending_counts_by_channel":["channel":1]]
    if let selectedRequest { state["selected_request_id"] = selectedRequest }
    if missingUser { state.removeValue(forKey: "current_user") }
    return String(decoding:try! JSONSerialization.data(withJSONObject:["ok":true,"result":state]),as:UTF8.self)
  }
}

final class NodAnswerStateTests: XCTestCase {
  @MainActor func testMissingPerUserIdentityPreservesUnknownWithoutWrite() async throws {
    let fixture = AnswerFixture(); await fixture.omitUser()
    await fixture.mode(readFails:false,resolved:true,perUser:true)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    store.answerAttempts["fixture:card"] = NodAnswerAttempt(serverId:"fixture",requestId:"card",optionId:"yes",optionLabel:"Yes",isSubmitting:true)
    let result = await store.checkAnswerStatus(requestId:"card",serverId:"fixture")
    XCTAssertFalse(result)
    XCTAssertEqual(store.answerAttempts["fixture:card"]?.isSubmitting,false)
    XCTAssertNotNil(store.lastError)
    XCTAssertNil(store.answerReceipt)
    let counts = await fixture.counts(); XCTAssertEqual(counts.0,0); XCTAssertEqual(counts.1,1)
  }

  @MainActor func testOrdinarySubmitForwardsRenderedDigestUsingRuntimeProfiles() async throws {
    let fixture = AnswerFixture()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    store.servers = [] // The UI mirror may lag; the applied runtime state is authoritative.
    XCTAssertFalse(runtime.state?.servers.isEmpty ?? true)
    let request = try XCTUnwrap(runtime.state?.requests.first)
    XCTAssertFalse(request.isValueRequest)
    XCTAssertTrue(NodStore.isAnswerable(request))
    _ = await store.answerFromUI(request:request, option:try XCTUnwrap(request.options.first), serverId:"fixture")
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0, 1)
    let submitted = await fixture.submittedJSON()
    let params = try XCTUnwrap(JSONSerialization.jsonObject(with:try XCTUnwrap(submitted)) as? [String:Any])
    XCTAssertEqual(params["expected_request_digest"] as? String, "rendered-digest", "Ordinary requests bind the rendered digest too")
    XCTAssertEqual(params["option_id"] as? String, "yes")
    XCTAssertNil(params["answer_value"])
  }

  @MainActor func testUnsignedRequestIsNotAnswerableAndNeverWrites() async throws {
    let fixture = AnswerFixture(); await fixture.omitSigning()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    let request = try XCTUnwrap(runtime.state?.requests.first)
    XCTAssertNil(request.signing)
    XCTAssertFalse(NodStore.isAnswerable(request))
    let answered = await store.answerFromUI(request:request, option:try XCTUnwrap(request.options.first))
    XCTAssertFalse(answered)
    XCTAssertEqual(store.lastError, "Request changed. Review it again before answering.")
    XCTAssertNil(store.answerAttempt(requestId:request.id))
    store.lastError = nil
    let submitted = await store.submit(request:request, option:try XCTUnwrap(request.options.first))
    XCTAssertFalse(submitted)
    XCTAssertNotNil(store.lastError)
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0, 0, "No digest to bind means nothing is sent for the runtime to sign")
  }

  @MainActor func testInformationalAcknowledgeForwardsRenderedDigest() async throws {
    let fixture = AnswerFixture(); await fixture.information()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    let request = try XCTUnwrap(runtime.state?.requests.first)
    XCTAssertTrue(request.options.isEmpty)
    _ = await store.dismissIfInformational(request:request)
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0, 1)
    let submitted = await fixture.submittedJSON()
    let params = try XCTUnwrap(JSONSerialization.jsonObject(with:try XCTUnwrap(submitted)) as? [String:Any])
    XCTAssertEqual(params["option_id"] as? String, "dismiss")
    XCTAssertEqual(params["expected_request_digest"] as? String, "rendered-digest")
    await fixture.omitSigning()
    await store.refresh()
    try await Task.sleep(for:.milliseconds(30))
    let unsigned = try XCTUnwrap(runtime.state?.requests.first)
    let acknowledgedUnsigned = await store.dismissIfInformational(request:unsigned)
    XCTAssertFalse(acknowledgedUnsigned)
    let after = await fixture.counts()
    XCTAssertEqual(after.0, 1)
  }

  func testValueComposerReopenUsesCanonicalNewNotes() {
    let detail = NodAnswerComposition.notesKey(serverId:"s",requestId:"r",optionId:"",isValueRequest:true)
    let composer = NodAnswerComposition.notesKey(serverId:"s",requestId:"r",optionId:"x",isValueRequest:true)
    var drafts = [composer:"old", "s:r:x":"obsolete"]
    drafts[detail] = "new"
    XCTAssertEqual(drafts[composer],"new")
    XCTAssertTrue(NodAnswerComposition.canSend(text:drafts[composer]!,isSubmitting:false))
    XCTAssertNotEqual(NodAnswerComposition.notesKey(serverId:"s",requestId:"r",optionId:"x",isValueRequest:false),detail)
  }
  @MainActor func testMacNextSkipsFilteredOutFirstAndSelectsMatchingPending() async throws {
    let fixture = AnswerFixture(); await fixture.includeOtherPending()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    store.setMacInboxFilter(pendingOnly:true,searchText:"Needle")
    XCTAssertEqual(store.macNextPending().map(\.id),["card"])
    store.selectNextMacPending()
    XCTAssertEqual(store.selectedRequestId,"card")
    XCTAssertEqual(store.macSearchText,"Needle")
    let counts = await fixture.counts(); XCTAssertEqual(counts.0,0)
  }

  @MainActor func testMacNextHandledModeHasNoEligiblePendingAndDoesNotChangeSelection() async throws {
    let fixture = AnswerFixture(); await fixture.includeOtherPending(); await fixture.mode(readFails:false,resolved:true)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    store.setMacInboxFilter(pendingOnly:false,searchText:"Needle"); store.setMacInboxMode(handled:true)
    store.selectRequestExplicitly("card")
    XCTAssertEqual(store.selectedRequestId,"card")
    XCTAssertTrue(store.macNextPending().isEmpty)
    store.selectNextMacPending()
    XCTAssertEqual(store.selectedRequestId,"card")
    XCTAssertTrue(store.macHandledOnly)
    XCTAssertEqual(store.macSearchText,"Needle")
    let counts = await fixture.counts(); XCTAssertEqual(counts.0,0)
  }

  @MainActor func testMacNextAfterReceiptWithNilFilteredSelectionSelectsOnlyEligiblePending() async throws {
    let fixture = AnswerFixture(); await fixture.includeOtherPending(matchingNext:true)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos,defaultDeviceName:"Synthetic",presentLocalNotifications:false,configureNotificationController:false,runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    store.setMacInboxFilter(pendingOnly:true,searchText:"Needle"); store.selectRequestExplicitly("card")
    await fixture.mode(readFails:false,resolved:true)
    _ = await store.checkAnswerStatus(requestId:"card",serverId:"fixture")
    try await Task.sleep(for:.milliseconds(30))
    XCTAssertNil(store.selectedRequestId)
    XCTAssertNotNil(store.answerReceipt)
    XCTAssertEqual(store.macNextPending().map(\.id),["next"])
    store.selectNextMacPending()
    XCTAssertEqual(store.selectedRequestId,"next")
    XCTAssertNil(store.answerReceipt)
    XCTAssertEqual(store.macSearchText,"Needle")
    XCTAssertTrue(store.macPendingOnly)
    let counts = await fixture.counts(); XCTAssertEqual(counts.0,0)
  }

  @MainActor func testValueWithoutOptionsNeverAcknowledges() async throws {
    let fixture = AnswerFixture()
    await fixture.information()
    await fixture.valueField(#"{"kind":"number","label":"Count","default":{"kind":"number","decimal":"4"}}"#)
    await fixture.mode(readFails:false,resolved:false)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos, defaultDeviceName:"Synthetic", presentLocalNotifications:false, configureNotificationController:false, runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    let request = try XCTUnwrap(runtime.state?.requests.first)
    XCTAssertTrue(request.isValueRequest)
    XCTAssertTrue(request.options.isEmpty)
    let acknowledged = await store.dismissIfInformational(request:request)
    XCTAssertFalse(acknowledged)
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0,0, "A value request without options is never dismissed on the user's behalf")
  }

  @MainActor func testValueNotesUTF8LimitBlocksBeforeAnyWrite() async throws {
    let fixture = AnswerFixture()
    await fixture.valueField(#"{"kind":"number","label":"Count","default":{"kind":"number","decimal":"4"}}"#)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
    let store = NodStore(platform:.macos, defaultDeviceName:"Synthetic", presentLocalNotifications:false, configureNotificationController:false, runtime:runtime)
    try await Task.sleep(for:.milliseconds(200))
    let request = try XCTUnwrap(runtime.state?.requests.first)
    _ = await store.answerFromUI(request:request,option:try XCTUnwrap(request.options.first),text:String(repeating:"🐈",count:4097),answerValue:.number("4"))
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0,0)
    XCTAssertNil(store.answerAttempt(requestId:"card"))
    _ = await store.answerFromUI(request:request,option:try XCTUnwrap(request.options.first),text:String(repeating:"🐈",count:4096),answerValue:.number("4"))
    let atBoundary = await fixture.counts()
    XCTAssertEqual(atBoundary.0,1)
  }

  @MainActor func testAllFourValueKindsForwardRenderedDigestAndNotesWithoutSwiftSigning() async throws {
    let definitions = [
      #"{"kind":"choice","label":"Batch","choices":[{"id":"small","label":"Small"},{"id":"large","label":"Large"}],"default":{"kind":"choice","choice_id":"small"}}"#,
      #"{"kind":"number","label":"Count","default":{"kind":"number","decimal":"4"}}"#,
      #"{"kind":"date","label":"Day","default":{"kind":"date","date":"2028-02-29"}}"#,
      #"{"kind":"date_time","label":"Time","default":{"kind":"date_time","instant":"2026-10-25T09:30:00Z"}}"#
    ]
    let selected: [NodAnswerValue] = [.choice("large"), .number("999999999999.123456"), .date("2026-10-26"), .dateTime("2026-10-26T11:45:00Z")]
    for (definition, value) in zip(definitions, selected) {
      let fixture = AnswerFixture(); await fixture.valueField(definition)
      let runtime = NodRuntimeClient(request: { await fixture.request($0) }); try await runtime.start()
      let store = NodStore(platform:.macos, defaultDeviceName:"Synthetic", presentLocalNotifications:false, configureNotificationController:false, runtime:runtime)
      try await Task.sleep(for:.milliseconds(200))
      let request = try XCTUnwrap(runtime.state?.requests.first)
      _ = await store.answerFromUI(request:request, option:try XCTUnwrap(request.options.first), text:"Notes 🐈", answerValue:value)
      let submitted = await fixture.submittedJSON()
      let data = try XCTUnwrap(submitted)
      let params = try XCTUnwrap(JSONSerialization.jsonObject(with:data) as? [String:Any])
      XCTAssertEqual(params["expected_request_digest"] as? String, "rendered-digest")
      XCTAssertEqual(params["text"] as? String, "Notes 🐈")
      let encoded = try JSONEncoder().encode(value)
      XCTAssertEqual(params["answer_value"] as? NSDictionary, try JSONSerialization.jsonObject(with:encoded) as? NSDictionary)
      XCTAssertNil(params["signature"], "Signing bytes must be built only by shared Rust")
      XCTAssertEqual(store.answerAttempt(requestId:request.id)?.answerValue, value)
    }
  }
  @MainActor func testCancelledCardLeavesPendingAndClearsSelectionWithoutWrite() async throws {
    let fixture = AnswerFixture()
    await fixture.information()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) })
    try await runtime.start()
    let store = NodStore(platform: .macos, defaultDeviceName: "Synthetic", presentLocalNotifications: false, configureNotificationController: false, runtime: runtime)
    try await Task.sleep(for: .milliseconds(200))
    store.initializeMacInboxUI()
    store.selectRequestExplicitly("card")
    XCTAssertEqual(store.selectedRequestId, "card")
    await fixture.cancel()
    await store.refresh()
    try await Task.sleep(for: .milliseconds(30))
    XCTAssertTrue(store.macInboxRequests.isEmpty)
    XCTAssertNil(store.selectedRequestId)
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0, 0)
  }
  @MainActor func testUnknownBlocksReplayAndReadCheckProducesSeparateReceipt() async throws {
    let fixture = AnswerFixture()
    let runtime = NodRuntimeClient(request: { await fixture.request($0) })
    try await runtime.start()
    let store = NodStore(platform: .macos, defaultDeviceName: "Synthetic", presentLocalNotifications: false, configureNotificationController: false, runtime: runtime)
    try await Task.sleep(for: .milliseconds(200))
    let request = try XCTUnwrap(runtime.state?.requests.first)
    let option = try XCTUnwrap(request.options.first)
    store.answerReceipt = NodAnswerReceipt(serverId:"fixture",request:request,decision:nil)
    _ = await store.answerFromUI(request: request, option: option)
    XCTAssertEqual(store.answerAttempt(requestId: request.id)?.optionLabel, "Yes")
    XCTAssertFalse(store.answerAttempt(requestId: request.id)?.isSubmitting ?? true)
    _ = await store.answerFromUI(request: request, option: option)
    let before = await fixture.counts()
    XCTAssertEqual(before.0, 1)
    XCTAssertNil(store.answerReceipt)
    await fixture.mode(readFails: false, resolved: true)
    _ = await store.checkAnswerStatus(requestId: request.id, serverId: "fixture")
    let after = await fixture.counts()
    XCTAssertEqual(after.0, 1)
    XCTAssertEqual(after.1, 2)
    XCTAssertEqual(after.2, 0, "Check status must not use the navigation-producing open_request RPC")
    XCTAssertNil(store.answerAttempt(requestId: request.id))
    XCTAssertEqual(store.answerReceipt?.decision?.optionId, "yes")
    XCTAssertNil(store.answerReceipt?.decision?.signature)
  }

  @MainActor func testPerUserReceiptDoesNotClaimSharedResolution() async throws {
    let fixture = AnswerFixture()
    await fixture.mode(readFails: false, resolved: true, perUser: true)
    let runtime = NodRuntimeClient(request: { await fixture.request($0) })
    try await runtime.start()
    let store = NodStore(platform: .macos, defaultDeviceName: "Synthetic", presentLocalNotifications: false, configureNotificationController: false, runtime: runtime)
    try await Task.sleep(for: .milliseconds(200))
    _ = await store.checkAnswerStatus(requestId: "card", serverId: "fixture")
    XCTAssertEqual(store.answerReceipt?.request.status, .pending)
    XCTAssertEqual(store.answerReceipt?.request.decisionResolution, .perUser)
    XCTAssertEqual(store.answerReceipt?.decision?.actorUserId, "me")
    let request = try XCTUnwrap(store.answerReceipt?.request)
    XCTAssertFalse(store.needsAnswer(request), "A personal answer belongs in Handled even while others remain pending")
    store.initializeMacInboxUI()
    XCTAssertTrue(store.macInboxRequests.isEmpty)
    store.currentUser = nil
    XCTAssertTrue(store.needsAnswer(request), "Missing user identity must not infer a personal answer")
    let counts = await fixture.counts()
    XCTAssertEqual(counts.0, 0)
  }

  func testLocalDraftRoundTripAndInterruptedAttemptIsUnknown() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions:0o700])
    let url = directory.appendingPathComponent("answer-ui.json")
    let attempt = NodAnswerAttempt(serverId:"server", requestId:"card", optionId:"notes", optionLabel:"Notes", isSubmitting:true)
    let state = NodAnswerLocalState(drafts:["server:card:notes":"Full\nanswer 🐈"], attempts:[attempt.key:attempt])
    try state.save(to:url)
    let loaded = try NodAnswerLocalState.load(from:url)
    XCTAssertEqual(loaded.drafts, state.drafts)
    XCTAssertEqual(loaded.attempts[attempt.key]?.isSubmitting, false)
    XCTAssertEqual((try FileManager.default.attributesOfItem(atPath:url.path)[.posixPermissions] as? NSNumber)?.intValue, 0o600)
  }
}
