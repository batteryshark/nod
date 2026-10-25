import Foundation

public struct NodAnswerAttempt: Codable, Equatable, Sendable {
  public let serverId: String
  public let requestId: String
  public let optionId: String
  public let optionLabel: String
  public var isSubmitting = false
  public var channelId: String? = nil
  public var requestTitle: String? = nil
  public var answerValue: NodAnswerValue? = nil
  public var key: String { serverId + ":" + requestId }
}

public struct NodAnswerReceipt: Identifiable, Sendable {
  public let serverId: String
  public let request: NodRequest
  public let decision: NodDecision?
  public var id: String { serverId + ":" + request.id }
}

/// UI content only, stored beside the runtime state, never inside its protocol.
/// No tokens, credentials, signing keys or executable/queued decisions.
struct NodAnswerLocalState: Codable {
  var drafts: [String: String] = [:]
  var attempts: [String: NodAnswerAttempt] = [:]
  var values: [String: NodAnswerValue] = [:]
  init(drafts: [String:String] = [:], attempts: [String:NodAnswerAttempt] = [:], values: [String:NodAnswerValue] = [:]) {
    self.drafts = drafts; self.attempts = attempts; self.values = values
  }
  private enum CodingKeys: String, CodingKey { case drafts, attempts, values }
  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    drafts = try c.decodeIfPresent([String:String].self, forKey: .drafts) ?? [:]
    attempts = try c.decodeIfPresent([String:NodAnswerAttempt].self, forKey: .attempts) ?? [:]
    values = try c.decodeIfPresent([String:NodAnswerValue].self, forKey: .values) ?? [:]
  }

  static func load(from url: URL) throws -> Self {
    guard FileManager.default.fileExists(atPath: url.path) else { return Self() }
    var state = try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    // Process interruption cannot prove that a write completed or never began.
    for key in state.attempts.keys { state.attempts[key]?.isSubmitting = false }
    return state
  }

  func save(to url: URL) throws {
    let data = try JSONEncoder().encode(self)
    #if os(iOS)
    try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    #else
    try data.write(to: url, options: .atomic)
    #endif
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    var resourceURL = url
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try resourceURL.setResourceValues(values)
  }
}

extension NodStore {
  public func prepareAnswerStorage() {
    // Test runtimes must never inspect or mutate installed application data.
    guard answerStorageURL == nil, answerStorageEnabled, let path = runtime.statePath else { return }
    let url = URL(fileURLWithPath: path).deletingLastPathComponent().appendingPathComponent("answer-ui.json")
    do {
      let state = try NodAnswerLocalState.load(from: url)
      responseDrafts = state.drafts
      answerAttempts = state.attempts
      answerValueDrafts = state.values
      answerStorageURL = url
      answerDraftsSaved = true
    } catch { answerStorageError = "Could not restore answer drafts on this device." }
  }

  func persistAnswerState() {
    guard let url = answerStorageURL else { answerDraftsSaved = false; return }
    do {
      try NodAnswerLocalState(drafts: responseDrafts, attempts: answerAttempts, values: answerValueDrafts).save(to: url)
      answerDraftsSaved = true
      answerStorageError = nil
    } catch {
      answerDraftsSaved = false
      answerStorageError = "Could not save answer drafts on this device. Keep Nod open."
    }
  }

  public func answerAttempt(requestId: String, serverId: String? = nil) -> NodAnswerAttempt? {
    guard let serverId = serverId ?? selectedServerId else { return nil }
    return answerAttempts[serverId + ":" + requestId]
  }

  public func recordedAnswer(in request: NodRequest) -> NodDecision? {
    if request.decisionResolution == .shared { return request.decision }
    guard let userId = currentUser?.id else { return nil }
    return request.decisions.first { $0.userId == userId }?.decision
  }

  public func needsAnswer(_ request: NodRequest) -> Bool {
    request.status == .pending && recordedAnswer(in: request) == nil
  }

  /// Presentation bookkeeping around the ONE existing runtime write path.
  @discardableResult
  public func answerFromUI(request: NodRequest, option: NodRequestOption, text: String? = nil, serverId: String? = nil, answerValue: NodAnswerValue? = nil) async -> Bool {
    guard let serverId = serverId ?? selectedServerId else { return false }
    guard NodStore.isAnswerable(request) else {
      lastError = "Request changed. Review it again before answering."
      return false
    }
    if request.isValueRequest {
      guard let field = request.answerField, let answerValue, field.isValid(answerValue),
        (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).utf8.count <= 16384 else {
        lastError = "Review the answer value and notes before sending."
        return false
      }
    }
    let attempt = NodAnswerAttempt(serverId: serverId, requestId: request.id, optionId: option.id, optionLabel: option.label, isSubmitting: true, channelId: request.channelId, requestTitle: request.title, answerValue: answerValue)
    prepareAnswerStorage()
    guard answerAttempts[attempt.key] == nil else { return false }
    answerReceipt = nil
    answerContext = NodAnswerReceipt(serverId: serverId, request: request, decision: nil)
    answerAttempts[attempt.key] = attempt
    _ = await submit(request: request, option: option, text: text, serverId: serverId, answerValue: answerValue)
    // HTTP/RPC success alone is not an authoritative receipt or verification.
    return await checkAnswerStatus(requestId: request.id, serverId: serverId)
  }

  @discardableResult
  public func checkAnswerStatus(requestId: String, serverId: String) async -> Bool {
    let key = serverId + ":" + requestId
    guard selectedServerId == serverId else {
      answerAttempts[key]?.isSubmitting = false
      return false
    }
    do {
      let userId = runtime.state?.currentUser?.id
      let request = try await runtime.getRequest(serverId: serverId, requestId: requestId)
      guard request.decisionResolution == .shared || userId != nil else {
        answerAttempts[key]?.isSubmitting = false
        lastError = "Could not check answer status without your user identity. Check status again when connected."
        return false
      }
      let decision: NodDecision?
      if request.decisionResolution == .shared { decision = request.decision }
      else { decision = request.decisions.first { $0.userId == userId }?.decision }
      if decision != nil || request.status != .pending {
        answerReceipt = NodAnswerReceipt(serverId: serverId, request: request, decision: decision)
        if answerContext?.id == key { answerContext = nil }
        if let attempt = answerAttempts[key], decision?.optionId == attempt.optionId,
          decision?.answerValue == attempt.answerValue,
          let userId, decision?.actorUserId == userId,
          (responseDrafts[NodAnswerComposition.notesKey(serverId: serverId, requestId: requestId, optionId: attempt.optionId, isValueRequest: request.isValueRequest)] ?? "").trimmingCharacters(in: .whitespacesAndNewlines) == (decision?.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines) {
          responseDrafts.removeValue(forKey: key + ":" + attempt.optionId)
          responseDrafts.removeValue(forKey: key + ":value-notes")
          answerValueDrafts = answerValueDrafts.filter { !$0.key.hasPrefix(key + ":") }
        }
        answerAttempts.removeValue(forKey: key)
        if selectedServerId == serverId { await refresh() }
        return decision != nil
      }
      // Authoritative pending state permits a NEW deliberate choice, never replay.
      answerAttempts.removeValue(forKey: key)
      return false
    } catch {
      answerAttempts[key]?.isSubmitting = false
      mapRuntimeError(error)
      return false
    }
  }
}
