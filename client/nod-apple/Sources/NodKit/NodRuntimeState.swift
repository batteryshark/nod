import Foundation

/// The decoded `ClientState` the shared Rust runtime (`nod-client-core`) emits
/// over the FFI — the same view type the Tauri desktop renders. NodKit decodes
/// it instead of maintaining a parallel store, reusing the wire models
/// (`NodChannel`/`NodUser`/`NodUserDevice`/`NodRequest`) that already match the
/// shared `nod-proto` shapes.
public struct NodRuntimeState: Codable, Sendable {
  public var servers: [NodRuntimeServerProfile]
  public var selectedServerId: String?
  public var currentUser: NodUser?
  public var devices: [NodUserDevice]
  public var channels: [NodChannel]
  public var pendingCountsByChannel: [String: Int]
  public var requests: [NodRequest]
  public var selectedChannelId: String?
  public var selectedRequestId: String?
  public var notificationSound: String
  public var notificationDeliveryMode: NodNotificationDeliveryMode
  public var isRegistered: Bool
  public var isSyncConnected: Bool
  public var syncPhase: String
  public var lastSyncedAt: Date?
  public var lastError: String?

  enum CodingKeys: String, CodingKey {
    case servers, channels, requests, devices
    case selectedServerId = "selected_server_id"
    case currentUser = "current_user"
    case pendingCountsByChannel = "pending_counts_by_channel"
    case selectedChannelId = "selected_channel_id"
    case selectedRequestId = "selected_request_id"
    case notificationSound = "notification_sound"
    case notificationDeliveryMode = "notification_delivery_mode"
    case isRegistered = "is_registered"
    case isSyncConnected = "is_sync_connected"
    case syncPhase = "sync_phase"
    case lastSyncedAt = "last_synced_at"
    case lastError = "last_error"
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    servers = try c.decodeIfPresent([NodRuntimeServerProfile].self, forKey: .servers) ?? []
    selectedServerId = try c.decodeIfPresent(String.self, forKey: .selectedServerId)
    currentUser = try c.decodeIfPresent(NodUser.self, forKey: .currentUser)
    devices = try c.decodeIfPresent([NodUserDevice].self, forKey: .devices) ?? []
    channels = try c.decodeIfPresent([NodChannel].self, forKey: .channels) ?? []
    pendingCountsByChannel =
      try c.decodeIfPresent([String: Int].self, forKey: .pendingCountsByChannel) ?? [:]
    requests = try c.decodeIfPresent([NodRequest].self, forKey: .requests) ?? []
    selectedChannelId = try c.decodeIfPresent(String.self, forKey: .selectedChannelId)
    selectedRequestId = try c.decodeIfPresent(String.self, forKey: .selectedRequestId)
    notificationSound = try c.decodeIfPresent(String.self, forKey: .notificationSound) ?? "default"
    notificationDeliveryMode =
      try c.decodeIfPresent(NodNotificationDeliveryMode.self, forKey: .notificationDeliveryMode)
      ?? .websocket
    isRegistered = try c.decodeIfPresent(Bool.self, forKey: .isRegistered) ?? false
    isSyncConnected = try c.decodeIfPresent(Bool.self, forKey: .isSyncConnected) ?? false
    syncPhase = try c.decodeIfPresent(String.self, forKey: .syncPhase) ?? (isSyncConnected ? "current" : "offline")
    lastSyncedAt = try c.decodeIfPresent(Date.self, forKey: .lastSyncedAt)
    lastError = try c.decodeIfPresent(String.self, forKey: .lastError)
  }

  public var totalPendingCount: Int {
    pendingCountsByChannel.values.reduce(0, +)
  }

  public var subscribedChannels: [NodChannel] {
    channels.filter(\.subscribed)
  }

  public var selectedServer: NodRuntimeServerProfile? {
    guard let selectedServerId else { return servers.first }
    return servers.first { $0.id == selectedServerId } ?? servers.first
  }
}

/// `ClientState.servers` entries — the runtime's `ServerProfile` (snake_case),
/// distinct from NodKit's locally-persisted `NodServerProfile`.
public struct NodRuntimeServerProfile: Codable, Identifiable, Hashable, Sendable {
  public var id: String
  public var name: String
  public var baseUrlString: String
  public var deviceName: String
  public var deviceId: String?
  public var userId: String?
  public var userName: String?
  public var credentialId: String?

  enum CodingKeys: String, CodingKey {
    case id, name
    case baseUrlString = "base_url_string"
    case deviceName = "device_name"
    case deviceId = "device_id"
    case userId = "user_id"
    case userName = "user_name"
    case credentialId = "credential_id"
  }
}

/// A decoded `NodClientMessage` envelope (`{ "kind": …, "payload": … }`) the
/// runtime pushes to its observer. Mirrors nod-client-core's `NodClientMessage`.
public enum NodRuntimeMessage: Sendable {
  case ready(statePath: String)
  case state(NodRuntimeState)
  case notificationCandidate(serverId: String?, request: NodRequest)
  case notificationRemoved(serverId: String?, requestId: String)
  case syncStatus(connected: Bool)
  case authRevoked
  case resyncRequired
  case transientError(message: String)

  public init(from data: Data) throws {
    // `.nod` carries the ISO-8601 date strategy the wire models require. Each
    // payload is decoded DIRECTLY into its concrete type from the original bytes
    // (no type-erased round-trip), so nothing is lost and a failure carries the
    // real underlying `DecodingError` rather than a generic "malformed".
    let decoder = JSONDecoder.nod
    let kind = try decoder.decode(KindEnvelope.self, from: data).kind
    switch kind {
    case "ready":
      self = .ready(
        statePath: (try? decoder.decode(Envelope<ReadyPayload>.self, from: data))?.payload.statePath
          ?? "")
    case "state":
      self = .state(try decoder.decode(Envelope<NodRuntimeState>.self, from: data).payload)
    case "notification_candidate":
      let payload = try decoder.decode(Envelope<NotificationCandidatePayload>.self, from: data).payload
      self = .notificationCandidate(serverId: payload.serverId, request: payload.request)
    case "notification_removed":
      let payload = try decoder.decode(Envelope<NotificationRemovedPayload>.self, from: data).payload
      self = .notificationRemoved(serverId: payload.serverId, requestId: payload.requestId)
    case "sync_status":
      self = .syncStatus(
        connected: try decoder.decode(Envelope<SyncStatusPayload>.self, from: data).payload
          .connected)
    case "auth_revoked":
      self = .authRevoked
    case "resync_required":
      self = .resyncRequired
    case "transient_error":
      self = .transientError(
        message: (try? decoder.decode(Envelope<TransientErrorPayload>.self, from: data))?.payload
          .message ?? "")
    default:
      throw NodRuntimeMessageError.unknownKind(kind)
    }
  }

  private struct KindEnvelope: Decodable { let kind: String }
  private struct Envelope<P: Decodable>: Decodable { let payload: P }

  private struct ReadyPayload: Decodable {
    let statePath: String
    enum CodingKeys: String, CodingKey { case statePath = "state_path" }
  }
  private struct NotificationCandidatePayload: Decodable {
    let request: NodRequest
    let serverId: String?
    enum CodingKeys: String, CodingKey { case request; case serverId = "server_id" }
  }
  private struct NotificationRemovedPayload: Decodable {
    let requestId: String
    let serverId: String?
    enum CodingKeys: String, CodingKey { case requestId = "request_id"; case serverId = "server_id" }
  }
  private struct SyncStatusPayload: Decodable { let connected: Bool }
  private struct TransientErrorPayload: Decodable { let message: String }
}

public enum NodRuntimeMessageError: Error {
  case malformed(String)
  case unknownKind(String)
}
