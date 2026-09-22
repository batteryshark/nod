import Foundation
import NodClientFFI

// The generated UniFFI `NodClient` wraps a thread-safe Rust object (`Arc<Mutex<…>>`),
// so it is safe to send across actors even though UniFFI 0.28's Swift bindings
// don't mark it `Sendable`.
extension NodClient: @unchecked Sendable {}

public enum NodRuntimeError: Error, LocalizedError {
  case notStarted
  case rpc(String)

  public var errorDescription: String? {
    switch self {
    case .notStarted: return "The Nod client runtime has not been started."
    case .rpc(let message): return message
    }
  }
}

/// The Apple-side driver for the shared `nod-client-core` runtime.
///
/// Owns a `NodClient` (the async Rust runtime over UniFFI), forwards its
/// `NodClientMessage` events onto the main actor as `@Published` state, and
/// sends RPCs in. This is what lets NodKit stop re-implementing the API client,
/// sync, store, and state machine — all of that now lives once in Rust (shared
/// with the TUI + desktop). The only Swift left is the native adapters: the
/// Secure Enclave signer (passed in), App Attest, UserNotifications, SwiftUI.
@MainActor
public final class NodRuntimeClient: ObservableObject {
  @Published public private(set) var state: NodRuntimeState?
  @Published public private(set) var statePath: String?
  @Published public private(set) var lastTransientError: String?
  @Published public private(set) var authRevoked: Bool = false

  /// Pending requests surfaced as local-notification candidates since the last
  /// time the host drained them.
  @Published public private(set) var notificationCandidates: [NodNotificationCandidate] = []
  @Published public private(set) var removedNotificationRequestIds: [NodNotificationTarget] = []

  private var client: NodClient?
  private let signer: any NodDeviceSigner & Sendable

  public init(signer: any NodDeviceSigner & Sendable = SecureEnclaveDeviceSigner()) {
    self.signer = signer
  }

  /// Build the runtime, attach the observer, and emit the initial state.
  public func start() async throws {
    guard client == nil else { return }
    let bridge = ObserverBridge { [weak self] message in
      Task { @MainActor in self?.apply(message) }
    }
    let client = try await NodClient(observer: bridge, signer: signer)
    self.client = client
    await client.start()
    try await callApplyingState("state")
  }

  // MARK: - RPCs (mirror nod-client-core's runtime methods)

  public func refresh() async throws { try await callApplyingState("refresh") }

  public func selectServer(_ serverId: String) async throws {
    try await callApplyingState("select_server", ["server_id": serverId])
  }

  public func forgetServer(_ serverId: String) async throws {
    try await callApplyingState("forget_server", ["server_id": serverId])
  }

  public func enroll(
    baseURL: String,
    deviceName: String,
    code: String,
    notificationSound: String?,
    platform: NodDevicePlatform,
    nativeAppId: String? = nil,
    pushProvider: String? = nil,
    pushToken: String? = nil,
    attestation: [String: Any]? = nil
  ) async throws {
    var params: [String: Any] = [
      "base_url": baseURL,
      "device_name": deviceName,
      "code": code,
      "platform": platform.rawValue,
    ]
    if let notificationSound { params["notification_sound"] = notificationSound }
    // Native enrollment hardening — forwarded to the server by the runtime
    // unchanged (the SE signing key is provisioned by the signer callback).
    if let nativeAppId { params["native_app_id"] = nativeAppId }
    if let pushProvider { params["push_provider"] = pushProvider }
    if let pushToken { params["push_token"] = pushToken }
    if let attestation { params["attestation"] = attestation }
    try await callApplyingState("enroll", params)
  }

  public func queryHistory(serverId: String?, channelId: String?, search: String, before: String?) async throws -> NodHistoryPage {
    var params: [String: Any] = ["search": search, "limit": 100]
    if let serverId { params["server_id"] = serverId }
    if let channelId { params["channel_id"] = channelId }
    if let before { params["before"] = before }
    let envelope = try JSONDecoder.nod.decode(HistoryEnvelope.self, from: try await send("query_history", params))
    guard envelope.ok, let page = envelope.result else { throw NodRuntimeError.rpc(envelope.error ?? "Could not load request history.") }
    return page
  }

  public func openRequest(serverId: String, requestId: String) async throws {
    try await callApplyingState("open_request", ["server_id": serverId, "request_id": requestId])
  }

  public func submitRequestOption(serverId: String, requestId: String, optionId: String, text: String?) async throws {
    var params: [String: Any] = ["server_id": serverId, "request_id": requestId, "option_id": optionId]
    if let text { params["text"] = text }
    try await call("submit_request_option", params)
  }

  public func submitOption(requestId: String, optionId: String, text: String?) async throws {
    var params: [String: Any] = ["request_id": requestId, "option_id": optionId]
    if let text { params["text"] = text }
    try await call("submit_option", params)
  }

  public func setSubscription(channelId: String, subscribed: Bool) async throws {
    try await callApplyingState(
      "set_subscription", ["channel_id": channelId, "subscribed": subscribed])
  }

  public func setNotificationPreference(sound: String) async throws {
    try await callApplyingState("set_notification_preference", ["notification_sound": sound])
  }

  public func setDeviceNotificationPreferences(_ preferences: NodDeviceNotificationPreferences, serverId: String) async throws {
    let encoded = try JSONEncoder.nod.encode(preferences)
    var params = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] ?? [:]
    params["server_id"] = serverId
    params["snoozed_until"] = params["snoozed_until"] ?? NSNull()
    try await callApplyingState("set_device_notification_preferences", params)
  }

  /// Register/refresh the APNs push token across every enrolled server.
  public func registerPushToken(provider: String, nativeAppId: String, token: String) async throws {
    try await callApplyingState(
      "register_push_token",
      ["provider": provider, "native_app_id": nativeAppId, "token": token])
  }

  public func clearChannel(_ channelId: String) async throws {
    try await callApplyingState("clear_channel", ["channel_id": channelId])
  }

  public func selectAllChannels() async throws { try await callApplyingState("select_all_channels") }

  public func selectChannel(_ channelId: String) async throws {
    try await callApplyingState("select_channel", ["channel_id": channelId])
  }

  public func selectRequest(_ requestId: String) async throws {
    try await callApplyingState("select_request", ["request_id": requestId])
  }

  public func renameDevice(deviceId: String, name: String) async throws {
    try await call("rename_device", ["device_id": deviceId, "name": name])
  }

  public func revokeDevice(_ deviceId: String) async throws {
    try await callApplyingState("revoke_device", ["device_id": deviceId])
  }

  public func connectSync() async throws { try await call("connect_sync") }
  public func disconnectSync() async throws { try await call("disconnect_sync") }

  /// Drain the queued notification candidates (the host shows them once).
  public func takeNotificationCandidates() -> [NodNotificationCandidate] {
    defer { notificationCandidates.removeAll() }
    return notificationCandidates
  }

  public func takeRemovedNotificationRequestIds() -> [NodNotificationTarget] {
    defer { removedNotificationRequestIds.removeAll() }
    return removedNotificationRequestIds
  }

  // MARK: - Internals

  private func send(_ method: String, _ params: [String: Any]) async throws -> Data {
    guard let client else { throw NodRuntimeError.notStarted }
    let body: [String: Any] = ["id": UUID().uuidString, "method": method, "params": params]
    let requestData = try JSONSerialization.data(withJSONObject: body)
    let responseJSON = await client.request(requestJson: String(decoding: requestData, as: UTF8.self))
    return Data(responseJSON.utf8)
  }

  /// Fire an RPC whose result the caller ignores; throws on a failure response.
  private func call(_ method: String, _ params: [String: Any] = [:]) async throws {
    let response = try JSONDecoder().decode(StatusEnvelope.self, from: try await send(method, params))
    guard response.ok else {
      throw NodRuntimeError.rpc(response.error ?? "request failed")
    }
  }

  /// Run a state-returning RPC (`enroll`, `refresh`, `select_server`, …) and
  /// apply the `ClientState` it returns directly — decoded straight into the
  /// type from the response bytes — so the UI updates synchronously with the
  /// call (mirroring the old hand-written client) instead of depending on the
  /// async observer event landing.
  private func callApplyingState(_ method: String, _ params: [String: Any] = [:]) async throws {
    let response = try JSONDecoder.nod.decode(
      StateEnvelope.self, from: try await send(method, params))
    guard response.ok else {
      throw NodRuntimeError.rpc(response.error ?? "request failed")
    }
    if let state = response.result {
      self.state = state
    }
  }

  private func apply(_ message: NodRuntimeMessage) {
    switch message {
    case .ready(let statePath):
      self.statePath = statePath
    case .state(let state):
      self.state = state
    case .notificationCandidate(let serverId, let request):
      notificationCandidates.append(NodNotificationCandidate(serverId: serverId, request: request))
    case .notificationRemoved(let serverId, let requestId):
      removedNotificationRequestIds.append(NodNotificationTarget(serverId: serverId, requestId: requestId))
    case .syncStatus:
      // Reflected in the next `state` event's `is_sync_connected`.
      break
    case .authRevoked:
      authRevoked = true
    case .resyncRequired:
      Task { try? await refresh() }
    case .transientError(let message):
      lastTransientError = message
    }
  }

  private struct HistoryEnvelope: Decodable {
    let ok: Bool
    let error: String?
    let result: NodHistoryPage?
  }

  private struct StatusEnvelope: Decodable {
    let ok: Bool
    let error: String?
  }

  private struct StateEnvelope: Decodable {
    let ok: Bool
    let error: String?
    let result: NodRuntimeState?
  }
}

/// Adapts the off-main-thread `NodClientObserver` callback into a decoded,
/// main-actor-bound stream. UniFFI invokes `onMessage` from Rust's pump thread;
/// we decode and hop to the main actor in the closure.
private final class ObserverBridge: NodClientObserver, @unchecked Sendable {
  private let onDecoded: @Sendable (NodRuntimeMessage) -> Void

  init(onDecoded: @escaping @Sendable (NodRuntimeMessage) -> Void) {
    self.onDecoded = onDecoded
  }

  func onMessage(message: String) {
    do {
      onDecoded(try NodRuntimeMessage(from: Data(message.utf8)))
    } catch {
      // Never silently swallow — surface as a transient error so drift is visible
      // instead of producing a "nothing happened" dead end.
      onDecoded(.transientError(message: "runtime event decode failed: \(error)"))
    }
  }
}

public struct NodHistoryPage: Decodable, Sendable {
  public let requests: [NodRequest]
  public let nextCursor: String?
  enum CodingKeys: String, CodingKey { case requests; case nextCursor = "next_cursor" }
}
