import Combine
import Foundation

public struct NodNotificationOpenRequest: Identifiable, Equatable, Sendable {
  public let id = UUID()
  public let requestId: String?
  public let channelId: String?
  public let serverId: String?
}

/// The SwiftUI-facing store. After the cutover onto the shared Rust runtime
/// (`nod-client-core`), this is a thin facade over `NodRuntimeClient`: the API
/// client, sync socket, persistence, and state machine all live once in Rust
/// (shared with the TUI + desktop). `NodStore` mirrors the runtime's emitted
/// `ClientState` into its existing `@Published` surface so the 9 SwiftUI views
/// compile unchanged, owns the UI-only inputs (URL/device-name/code drafts,
/// notification-permission status), and forwards user actions in as RPCs.
@MainActor
public final class NodStore: ObservableObject {
  // MARK: Data-plane (mirrored from the runtime's ClientState)
  @Published public var servers: [NodServerProfile] = []
  @Published public var selectedServerId: String?
  @Published public var currentUser: NodUser?
  @Published public var registeredDevices: [NodUserDevice] = []
  @Published public var channels: [NodChannel] = []
  @Published public var pendingCountsByChannel: [String: Int] = [:]
  @Published public var requests: [NodRequest] = []
  @Published public var notificationSound: String
  @Published public var lastError: String?
  @Published public var isRegistered: Bool = false
  @Published public var isSyncConnected: Bool = false
  @Published public var syncPhase = "offline"
  @Published public var lastSyncedAt: Date?
  @Published public var pendingActions = Set<String>()
  @Published public var responseDrafts: [String: String] = [:]
  @Published public var isRegistering = false
  @Published public var isRefreshing = false
  @Published public var acknowledgeOnOpen = UserDefaults.standard.object(forKey: "nod.acknowledgeOnOpen") as? Bool ?? true {
    didSet { defaults.set(acknowledgeOnOpen, forKey: "nod.acknowledgeOnOpen") }
  }
  @Published public private(set) var deviceNotificationPreferences = NodDeviceNotificationPreferences()
  public var isUpdatingNotificationPreferences: Bool {
    pendingActions.contains("notification_preferences:" + (selectedServerId ?? ""))
  }
  @Published public internal(set) var notificationDeliveryMode: NodNotificationDeliveryMode = .push

  // The selection is owned by the UI (views bind to it directly) but is also
  // forwarded into the runtime so the shared state machine tracks it. The
  // `didSet` guards against feedback loops when we mirror runtime state back in.
  @Published public var selectedChannelId: String? {
    didSet {
      guard !isApplyingRuntimeState, selectedChannelId != oldValue else { return }
      if let selectedChannelId {
        let channelId = selectedChannelId
        Task {
          do { try await runtime.selectChannel(channelId) } catch { mapRuntimeError(error) }
        }
      } else {
        Task {
          do { try await runtime.selectAllChannels() } catch { mapRuntimeError(error) }
        }
      }
      recomputeVisibleRequests()
    }
  }
  @Published public var selectedRequestId: String? {
    didSet {
      guard !isApplyingRuntimeState, selectedRequestId != oldValue else { return }
      if let selectedRequestId {
        let requestId = selectedRequestId
        Task { try? await runtime.selectRequest(requestId) }
      }
    }
  }

  // MARK: UI-only inputs (never sourced from the runtime)
  @Published public var baseURLString: String
  @Published public var deviceName: String
  @Published public var enrollmentCode: String = ""
  @Published public var notificationPermissionIssue: String?
  @Published public var notificationAuthorizationStatus: NodNotificationAuthorizationStatus =
    .notDetermined
  @Published public var notificationOpenRequest: NodNotificationOpenRequest?
  @Published public var registrationPromptRequestId: UUID?
  @Published public internal(set) var reEnrollmentServerId: String?
  @Published public internal(set) var serverConnectionIssuesById: [String: String] = [:]

  public let platform: NodDevicePlatform
  public var presentLocalNotifications: Bool

  public var selectedServer: NodServerProfile? {
    guard let selectedServerId else {
      return servers.first
    }
    return servers.first { $0.id == selectedServerId } ?? servers.first
  }

  public var subscribedChannels: [NodChannel] {
    channels.filter(\.subscribed)
  }

  public var totalPendingCount: Int {
    pendingCountsByChannel.values.reduce(0, +)
  }

  public var alertMessage: String? {
    lastError ?? notificationPermissionIssue
  }

  public var canReEnrollInvalidSession: Bool {
    reEnrollmentServerId != nil
  }

  public func connectionIssue(for server: NodServerProfile) -> String? {
    serverConnectionIssuesById[server.id]
  }

  public static let notificationSoundOptions: [NodNotificationSoundOption] = [
    NodNotificationSoundOption(id: "default", label: "Default"),
    NodNotificationSoundOption(id: "nod_ping.wav", label: "Ping"),
    NodNotificationSoundOption(id: "nod_chime.wav", label: "Chime"),
    NodNotificationSoundOption(id: "nod_low.wav", label: "Low"),
    NodNotificationSoundOption(id: "silent", label: "Silent"),
  ]

  static let applePushProvider = "apple_apns"

  let signingKeys = NodSigningKeyStore()
  let appAttest: NodAppAttestationProviding
  let defaults = UserDefaults.standard

  /// The shared Rust runtime that now owns the entire client.
  let runtime: NodRuntimeClient
  private var cancellables = Set<AnyCancellable>()
  /// All requests visible across channels (the runtime's full snapshot), kept so
  /// `selectedChannelId` changes can re-filter without a round trip.
  private var allVisibleRequests: [NodRequest] = []
  /// Set while mirroring runtime state in, so `selectedChannelId`/`Id` `didSet`
  /// observers don't echo the change back into the runtime.
  private var isApplyingRuntimeState = false
  private var hasStarted = false
  private var startupTask: Task<Void, Error>?
  /// Pending request ids already turned into a local notification, so a backlog
  /// isn't replayed as a burst. The runtime de-dups candidates, this guards a
  /// second time across app restarts within a session.
  var presentedNotificationRequestIds = Set<String>()
  private var reconciledNotificationIds = Set<String>()
  /// The latest APNs token, cached so re-enrollment can forward it again.
  var pushToken: String?

  public init(
    platform: NodDevicePlatform,
    defaultDeviceName: String,
    presentLocalNotifications: Bool,
    appAttest: NodAppAttestationProviding = NodAppAttestationStore(),
    configureNotificationController: Bool = true
  ) {
    self.platform = platform
    self.presentLocalNotifications = presentLocalNotifications
    self.appAttest = appAttest

    let savedDeviceName = defaults.string(forKey: "nod.deviceName") ?? defaultDeviceName
    self.baseURLString = ""
    self.deviceName = savedDeviceName
    self.notificationSound = defaults.string(forKey: "nod.notificationSound") ?? "default"

    self.runtime = NodRuntimeClient(signer: SecureEnclaveDeviceSigner(store: signingKeys))

    if configureNotificationController {
      NodNotificationController.shared.configure(
        onOpen: { [weak self] target in
          Task { @MainActor in await self?.openNotification(target) }
        },
        onOption: { [weak self] target, optionId, text in
          await self?.submitNotificationOption(target: target, optionId: optionId, text: text) ?? false
        }
      )
    }

    subscribeToRuntime()
    Task { await self.startRuntimeIfNeeded() }
  }

  // MARK: - Runtime lifecycle

  func startRuntimeIfNeeded() async {
    guard !hasStarted else { return }
    hasStarted = true
    do {
      try await ensureRuntimeStarted()
      try? await runtime.refresh()
      // Open the realtime sync socket if a server is enrolled. Without this the
      // app gets no push of new/resolved requests — no badge, no local
      // notification, and a stale request list (which then 409s on submit).
      if runtime.state?.isRegistered == true {
        connectSync()
      }
    } catch {
      hasStarted = false
      mapRuntimeError(error)
    }
  }

  func ensureRuntimeStarted() async throws {
    if let startupTask { return try await startupTask.value }
    let runtime = self.runtime
    let task = Task { try await runtime.start() }
    startupTask = task
    do { try await task.value } catch { startupTask = nil; throw error }
  }

  private func subscribeToRuntime() {
    runtime.$state
      .receive(on: DispatchQueue.main)
      .sink { [weak self] state in
        guard let self, let state else { return }
        self.apply(runtimeState: state)
      }
      .store(in: &cancellables)

    runtime.$notificationCandidates
      .receive(on: DispatchQueue.main)
      .sink { [weak self] candidates in
        guard let self, !candidates.isEmpty else { return }
        let drained = self.runtime.takeNotificationCandidates()
        Task { @MainActor in await self.presentNotificationCandidates(drained) }
      }
      .store(in: &cancellables)

    runtime.$removedNotificationRequestIds
      .receive(on: DispatchQueue.main)
      .sink { [weak self] removed in
        guard let self, !removed.isEmpty else { return }
        let drained = self.runtime.takeRemovedNotificationRequestIds()
        for target in drained {
          self.presentedNotificationRequestIds.remove(target.notificationId)
        }
        let targets = drained.map { target in
          NodNotificationTarget(serverId: target.serverId, deviceId: self.servers.first { $0.id == target.serverId }?.deviceId, requestId: target.requestId)
        }
        Task { await NodNotificationController.shared.removeNotifications(for: targets) }
      }
      .store(in: &cancellables)

    runtime.$authRevoked
      .receive(on: DispatchQueue.main)
      .sink { [weak self] revoked in
        guard let self, revoked else { return }
        self.handleAuthRevoked()
      }
      .store(in: &cancellables)

    runtime.$lastTransientError
      .receive(on: DispatchQueue.main)
      .compactMap { $0 }
      .sink { [weak self] message in
        self?.lastError = message
      }
      .store(in: &cancellables)
  }

  /// Mirror the runtime's `ClientState` into the view-facing surface.
  private func apply(runtimeState state: NodRuntimeState) {
    isApplyingRuntimeState = true
    defer { isApplyingRuntimeState = false }

    servers = state.servers.map { profile in
      NodServerProfile(
        id: profile.id,
        name: profile.name,
        baseURLString: profile.baseUrlString,
        deviceName: profile.deviceName,
        deviceId: profile.deviceId,
        userId: profile.userId,
        userName: profile.userName,
        credentialId: profile.credentialId
      )
    }
    serverConnectionIssuesById = serverConnectionIssuesById.filter { issue in
      servers.contains { $0.id == issue.key }
    }
    if state.syncPhase == "current", let serverId = state.selectedServerId {
      serverConnectionIssuesById.removeValue(forKey: serverId)
    }
    selectedServerId = state.selectedServerId
    currentUser = state.currentUser
    registeredDevices = state.devices
    deviceNotificationPreferences = state.devices.first { $0.isCurrent }?.notificationPreferences ?? .init()
    channels = state.channels
    pendingCountsByChannel = state.pendingCountsByChannel
    notificationSound = state.notificationSound
    notificationDeliveryMode = state.notificationDeliveryMode
    isRegistered = state.isRegistered
    isSyncConnected = state.isSyncConnected
    syncPhase = state.syncPhase
    lastSyncedAt = state.lastSyncedAt

    if selectedChannelId != state.selectedChannelId {
      selectedChannelId = state.selectedChannelId
    }
    if selectedRequestId != state.selectedRequestId {
      selectedRequestId = state.selectedRequestId
    }

    allVisibleRequests = state.requests
    recomputeVisibleRequests()
    if let serverId = state.selectedServerId {
      let deviceId = state.servers.first { $0.id == serverId }?.deviceId
      let completed = state.requests.filter { $0.status != .pending }.compactMap { request -> NodNotificationTarget? in
        let key = serverId + ":" + request.id
        guard reconciledNotificationIds.insert(key).inserted else { return nil }
        return NodNotificationTarget(serverId: serverId, deviceId: deviceId, requestId: request.id)
      }
      if !completed.isEmpty { Task { await NodNotificationController.shared.removeNotifications(for: completed) } }
    }

    if let error = state.lastError {
      lastError = error
    }
  }

  /// The views expect `requests` to be the selected channel's visible items.
  private func recomputeVisibleRequests() {
    guard let selectedChannelId else {
      requests = NodRequestInbox.visibleRequests(allVisibleRequests)
      return
    }
    requests = NodRequestInbox.visibleRequests(
      allVisibleRequests.filter { $0.channelId == selectedChannelId }
    )
  }

  public func importEnrollmentLink(_ url: URL) {
    guard !isRegistering else { return }
    do {
      let link = try NodEnrollmentLink(url: url)
      baseURLString = link.serverURL
      enrollmentCode = link.code
      if let name = link.deviceName, !name.isEmpty { deviceName = name }
      registrationPromptRequestId = UUID()
      lastError = nil
    } catch { mapRuntimeError(error) }
  }

  // MARK: - View-facing actions

  public func dismissAlertMessage() {
    if lastError != nil {
      lastError = nil
      reEnrollmentServerId = nil
    } else {
      notificationPermissionIssue = nil
    }
  }

  public func selectServer(_ serverId: String) {
    guard selectedServerId != serverId else {
      return
    }
    selectedServerId = serverId
    Task {
      do { try await runtime.selectServer(serverId) } catch { mapRuntimeError(error) }
    }
  }

  public func refresh() async {
    guard !isRefreshing else { return }
    isRefreshing = true
    defer { isRefreshing = false }
    do {
      try await ensureRuntimeStarted()
      try await runtime.refresh()
      lastError = nil
    } catch {
      mapRuntimeError(error)
    }
  }

  /// Account/device metadata lives in the same `ClientState`, so a plain refresh
  /// is enough; kept as a separate method to preserve the view surface.
  public func queryHistory(serverId: String?, channelId: String?, search: String, before: String?) async throws -> NodHistoryPage {
    try await ensureRuntimeStarted()
    return try await runtime.queryHistory(serverId: serverId, channelId: channelId, search: search, before: before)
  }

  public func refreshAccount() async {
    await refresh()
  }

  public func resumeFromForeground() async {
    guard isRegistered else {
      return
    }
    await refresh()
    connectSync()
  }

  public func connectSync() {
    Task { try? await runtime.connectSync() }
  }

  public func disconnectSync() {
    Task { try? await runtime.disconnectSync() }
  }

  @discardableResult
  public func submit(request: NodRequest, option: NodRequestOption, text: String? = nil, serverId: String? = nil) async -> Bool {
    guard let serverId = serverId ?? selectedServer?.id else { return false }
    return await submitNotificationOption(target: NodNotificationTarget(serverId: serverId, requestId: request.id, channelId: request.channelId), optionId: option.id, text: text)
  }

  @discardableResult
  public func dismissIfInformational(request: NodRequest, serverId: String? = nil) async -> Bool {
    guard request.status == .pending, request.options.isEmpty, let serverId = serverId ?? selectedServer?.id else { return false }
    let target = NodNotificationTarget(serverId: serverId, requestId: request.id, channelId: request.channelId)
    return await submitNotificationOption(target: target, optionId: "dismiss", text: nil)
  }

  /// Refresh when a submit fails because the local request list is stale (the
  /// request was resolved/expired elsewhere) — a 404/409 from the server.
  private func reconcileIfStale(_ error: Error) async {
    guard case NodRuntimeError.rpc(let message) = error else { return }
    let lower = message.lowercased()
    if lower.contains("conflict") || lower.contains("409") || lower.contains("404")
      || lower.contains("no longer pending") || lower.contains("not found")
    {
      try? await runtime.refresh()
    }
  }

  public func setSubscription(channelId: String, subscribed: Bool) async {
    do {
      try await runtime.setSubscription(channelId: channelId, subscribed: subscribed)
      lastError = nil
    } catch {
      mapRuntimeError(error)
    }
  }

  public func setNotificationSound(_ sound: String) async {
    let previousSound = notificationSound
    notificationSound = sound
    defaults.set(sound, forKey: "nod.notificationSound")
    do {
      try await runtime.setNotificationPreference(sound: sound)
      lastError = nil
    } catch {
      notificationSound = previousSound
      defaults.set(previousSound, forKey: "nod.notificationSound")
      mapRuntimeError(error)
    }
  }

  @discardableResult
  public func setDeviceNotificationPreferences(_ preferences: NodDeviceNotificationPreferences, serverId: String? = nil) async -> Bool {
    guard let serverId = serverId ?? selectedServerId else { return false }
    let action = "notification_preferences:" + serverId
    guard pendingActions.insert(action).inserted else { return false }
    defer { pendingActions.remove(action) }
    do {
      try await runtime.setDeviceNotificationPreferences(preferences, serverId: serverId)
      if selectedServerId == serverId { deviceNotificationPreferences = preferences }
      let deviceId = servers.first { $0.id == serverId }?.deviceId
      let targets: [NodNotificationTarget]
      if preferences.hideContent || preferences.snoozedUntil.map({ $0 > Date() }) == true {
        targets = [NodNotificationTarget(serverId: serverId, deviceId: deviceId)]
      } else {
        targets = preferences.mutedChannels.map { NodNotificationTarget(serverId: serverId, deviceId: deviceId, channelId: $0) }
      }
      if !targets.isEmpty { await NodNotificationController.shared.removeNotifications(for: targets) }
      lastError = nil
      return true
    } catch {
      mapRuntimeError(error)
      return false
    }
  }

  public func clearSelectedChannel() async {
    guard let selectedChannelId else {
      return
    }
    do {
      try await runtime.clearChannel(selectedChannelId)
      lastError = nil
    } catch {
      mapRuntimeError(error)
    }
  }

  @discardableResult
  public func renameDevice(_ device: NodUserDevice, name: String) async -> Bool {
    let action = "rename:" + device.id
    guard pendingActions.insert(action).inserted else { return false }
    defer { pendingActions.remove(action) }
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
      return false
    }
    do {
      try await runtime.renameDevice(deviceId: device.id, name: trimmed)
      lastError = nil
      return true
    } catch {
      mapRuntimeError(error)
      return false
    }
  }

  @discardableResult
  public func revokeDevice(_ device: NodUserDevice) async -> Bool {
    let server = selectedServer
    let action = "revoke:" + device.id
    guard pendingActions.insert(action).inserted else { return false }
    defer { pendingActions.remove(action) }
    do {
      try await runtime.revokeDevice(device.id)
      if device.isCurrent, let server {
        await NodNotificationController.shared.removeNotifications(for: NodNotificationTarget(serverId: server.id, deviceId: device.id))
      }
      lastError = nil
      return true
    } catch {
      mapRuntimeError(error)
      return false
    }
  }

  public func revokeCurrentDevice() async {
    guard let server = selectedServer, let deviceId = server.deviceId else {
      return
    }
    do {
      try await runtime.revokeDevice(deviceId)
      lastError = nil
    } catch {
      mapRuntimeError(error)
    }
  }

  public func forgetServers(_ serverIds: [String]) {
    for serverId in serverIds {
      let server = servers.first { $0.id == serverId }
      responseDrafts = responseDrafts.filter { !$0.key.hasPrefix(serverId + ":") }
      try? appAttest.delete(account: Self.appAttestKeyAccount(for: server?.credentialId ?? serverId))
      Task { await NodNotificationController.shared.removeNotifications(for: NodNotificationTarget(serverId: serverId, deviceId: server?.deviceId)) }
    }
    Task {
      for serverId in serverIds {
        do { try await runtime.forgetServer(serverId) } catch { mapRuntimeError(error) }
      }
    }
  }

  public func beginInvalidSessionReEnrollment() {
    guard
      let serverId = reEnrollmentServerId,
      let server = servers.first(where: { $0.id == serverId })
    else {
      reEnrollmentServerId = nil
      return
    }

    let shouldPromptRegistration = servers.contains { $0.id != server.id }
    baseURLString = server.baseURLString
    deviceName = server.deviceName
    enrollmentCode = ""
    lastError = nil
    reEnrollmentServerId = nil
    forgetServers([server.id])

    if shouldPromptRegistration {
      registrationPromptRequestId = UUID()
    }
  }

  /// Enroll the current draft (URL/device-name/code) with the server.
  ///
  /// App Attest still runs natively here, before handing off to the runtime: the
  /// Secure Enclave decision-signing key is provisioned locally (the runtime's
  /// signer callback uses the same keychain account, so it sees the same key),
  /// its public key feeds the App Attest `clientDataHash`, and the resulting
  /// attestation blob is forwarded to the runtime's `enroll` RPC.
  @discardableResult
  public func register(pushToken: String? = nil) async -> Bool {
    guard !isRegistering else { return false }
    isRegistering = true
    defer { isRegistering = false }
    do {
      try await ensureRuntimeStarted()
      guard !baseURLString.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        throw NodStoreError.invalidServerURL
      }

      let normalizedURL = NodServerAddress.normalizedBaseURL(baseURLString)
      guard URL(string: normalizedURL) != nil else {
        throw NodStoreError.invalidServerURL
      }

      let registrationPushToken = pushToken ?? self.pushToken
      let nativeAppId = try Self.nativeAppId(requiredForPushToken: registrationPushToken)
      let profileId = NodServerAddress.profileId(for: normalizedURL)
      // Load-or-create the Secure Enclave signing key for this profile. The
      // runtime's signer callback keys off the same account, so this mints the
      // exact key the runtime will use to sign decisions.
      let signingKey = try signingKeys.signingKey(account: NodSigningKeyStore.account(for: profileId))
      let attestationRequest = NodAppAttestationRequest(
        code: normalizedEnrollmentCode,
        deviceName: deviceName,
        platform: platform,
        pushProvider: registrationPushToken == nil ? nil : Self.applePushProvider,
        pushToken: registrationPushToken,
        signingKey: signingKey,
        account: Self.appAttestKeyAccount(for: profileId)
      )
      // App Attest hardens enrollment when Apple can issue an attestation, but the
      // Secure Enclave decision-signing key remains the required device identity.
      let attestation = try? await appAttest.enrollmentAttestation(for: attestationRequest)

      try await runtime.enroll(
        baseURL: normalizedURL,
        deviceName: deviceName,
        code: normalizedEnrollmentCode,
        notificationSound: notificationSound,
        platform: platform,
        nativeAppId: nativeAppId,
        pushProvider: registrationPushToken == nil ? nil : Self.applePushProvider,
        pushToken: registrationPushToken,
        attestation: attestation.map(Self.attestationDictionary)
      )
      enrollmentCode = ""
      lastError = nil
      // Start realtime sync immediately after enrolling so notifications and
      // badge counts work without waiting for a relaunch.
      connectSync()
      return true
    } catch {
      mapRuntimeError(error)
      return false
    }
  }

  public func registerPushToken(_ token: String) async {
    pushToken = token
    guard let nativeAppId = try? Self.nativeAppId(requiredForPushToken: token) else {
      lastError = NodStoreError.missingNativeAppId.localizedDescription
      return
    }
    do {
      try await ensureRuntimeStarted()
      try await runtime.registerPushToken(
        provider: Self.applePushProvider, nativeAppId: nativeAppId, token: token)
      lastError = nil
    } catch {
      mapRuntimeError(error)
    }
  }

  private static func attestationDictionary(_ attestation: NodDeviceAttestation) -> [String: Any] {
    [
      "provider": attestation.provider,
      "key_id": attestation.keyId,
      "attestation_object": attestation.attestationObject,
    ]
  }

  func submitNotificationOption(target: NodNotificationTarget, optionId: String, text: String?) async -> Bool {
    guard let requestId = target.requestId else { return false }
    do {
      try await ensureRuntimeStarted()
      // Read the directly applied runtime state; the UI mirror may still be queued after cold start.
      let profiles = runtime.state?.servers.map { NodServerProfile(id: $0.id, name: $0.name, baseURLString: $0.baseUrlString, deviceName: $0.deviceName, deviceId: $0.deviceId) } ?? servers
      guard let serverId = NodNotificationPolicy.serverId(for: target, servers: profiles) else {
        throw NodStoreError.ambiguousNotificationServer
      }
      let key = serverId + ":" + requestId
      guard pendingActions.insert(key).inserted else { return false }
      defer { pendingActions.remove(key) }
      try await runtime.submitRequestOption(serverId: serverId, requestId: requestId, optionId: optionId, text: text)
      await NodNotificationController.shared.removeNotifications(for: NodNotificationTarget(serverId: serverId, deviceId: target.deviceId ?? profiles.first { $0.id == serverId }?.deviceId, requestId: requestId))
      lastError = nil
      return true
    } catch {
      mapRuntimeError(error)
      await reconcileIfStale(error)
      return false
    }
  }

  public func isSubmitting(_ requestId: String) -> Bool {
    pendingActions.contains { $0.hasSuffix(":" + requestId) }
  }

  // MARK: - Error mapping / auth

  func mapRuntimeError(_ error: Error) {
    switch error {
    case NodRuntimeError.rpc(let message):
      lastError = message
    default:
      lastError = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
    }
  }

  private func handleAuthRevoked() {
    if let server = selectedServer {
      let message =
        "Your Nod session with \(server.name) is no longer valid. Re-enroll this device to continue."
      reEnrollmentServerId = server.id
      var issues = serverConnectionIssuesById
      issues[server.id] = message
      serverConnectionIssuesById = issues
      lastError = message
    } else {
      lastError = "Your Nod session is no longer valid. Re-enroll this device to continue."
      reEnrollmentServerId = nil
    }
  }

  // MARK: - Keychain namespaces

  static func appAttestKeyAccount(for serverId: String) -> String {
    "appAttestKey.\(serverId)"
  }

  static func nativeAppId(requiredForPushToken pushToken: String?) throws -> String? {
    let nativeAppId = Bundle.main.bundleIdentifier?
      .trimmingCharacters(in: .whitespacesAndNewlines)
    guard let nativeAppId, !nativeAppId.isEmpty else {
      if pushToken == nil {
        return nil
      }
      throw NodStoreError.missingNativeAppId
    }
    return nativeAppId
  }

  var normalizedEnrollmentCode: String {
    enrollmentCode.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
  }
}

public enum NodStoreError: Error, LocalizedError {
  case missingNativeAppId
  case invalidServerURL
  case ambiguousNotificationServer
  case invalidEnrollmentLink

  public var errorDescription: String? {
    switch self {
    case .missingNativeAppId:
      return "This app is missing a bundle identifier for push registration."
    case .invalidServerURL:
      return "The Nod server URL is invalid."
    case .invalidEnrollmentLink:
      return "This setup link is invalid. Use a Nod enrollment link containing a server address and an 8-character code."
    case .ambiguousNotificationServer:
      return "This notification does not identify an enrolled server. Open the request from its server in Nod to respond safely."
    }
  }
}
