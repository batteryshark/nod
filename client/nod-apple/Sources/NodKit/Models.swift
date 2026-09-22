import Foundation

public enum NodDevicePlatform: String, Codable, Sendable {
  case ios
  case macos
  case watchos
  case windows
  case linux
  case unknown
}

public struct NodChannel: Codable, Identifiable, Hashable, Sendable {
  public var id: String
  public var name: String
  public var emoji: String
  public var subscribed: Bool
  public var createdAt: Date

  enum CodingKeys: String, CodingKey {
    case id, name, emoji, subscribed
    case createdAt = "created_at"
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    id = try container.decode(String.self, forKey: .id)
    name = try container.decode(String.self, forKey: .name)
    emoji = try container.decode(String.self, forKey: .emoji)
    subscribed = try container.decodeIfPresent(Bool.self, forKey: .subscribed) ?? true
    createdAt = try container.decode(Date.self, forKey: .createdAt)
  }
}

public struct NodServerProfile: Codable, Identifiable, Hashable, Sendable {
  public var id: String
  public var name: String
  public var baseURLString: String
  public var deviceName: String
  public var deviceId: String?
  public var userId: String?
  public var userName: String?
  public var credentialId: String?

  public init(
    id: String,
    name: String,
    baseURLString: String,
    deviceName: String,
    deviceId: String? = nil,
    userId: String? = nil,
    userName: String? = nil,
    credentialId: String? = nil
  ) {
    self.id = id
    self.name = name
    self.baseURLString = baseURLString
    self.deviceName = deviceName
    self.deviceId = deviceId
    self.userId = userId
    self.userName = userName
    self.credentialId = credentialId
  }
}

public struct NodUser: Codable, Identifiable, Hashable, Sendable {
  public let id: String
  public let name: String
  public let createdAt: Date
  public let updatedAt: Date

  enum CodingKeys: String, CodingKey {
    case id, name
    case createdAt = "created_at"
    case updatedAt = "updated_at"
  }
}

public struct NodDeviceNotificationPreferences: Codable, Hashable, Sendable {
  public var hideContent: Bool
  public var mutedChannels: [String]
  public var snoozedUntil: Date?

  public init(hideContent: Bool = false, mutedChannels: [String] = [], snoozedUntil: Date? = nil) {
    self.hideContent = hideContent
    self.mutedChannels = mutedChannels
    self.snoozedUntil = snoozedUntil
  }

  enum CodingKeys: String, CodingKey {
    case hideContent = "hide_content"
    case mutedChannels = "muted_channels"
    case snoozedUntil = "snoozed_until"
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    hideContent = try container.decodeIfPresent(Bool.self, forKey: .hideContent) ?? false
    mutedChannels = try container.decodeIfPresent([String].self, forKey: .mutedChannels) ?? []
    snoozedUntil = try container.decodeIfPresent(Date.self, forKey: .snoozedUntil)
  }
}

public struct NodUserDevice: Codable, Identifiable, Hashable, Sendable {
  public let id: String
  public let userId: String
  public var name: String
  public let platform: NodDevicePlatform
  public let nativeAppId: String?
  public let pushProvider: String?
  public let hasPushToken: Bool
  public let hasSigningKey: Bool
  public let notificationSound: String
  public let notificationPreferences: NodDeviceNotificationPreferences
  public let attestation: NodDeviceAttestationSummary?
  public let lastSeenAt: Date
  public let createdAt: Date
  public let isCurrent: Bool

  enum CodingKeys: String, CodingKey {
    case id, name, platform
    case userId = "user_id"
    case nativeAppId = "native_app_id"
    case pushProvider = "push_provider"
    case hasPushToken = "has_push_token"
    case hasSigningKey = "has_signing_key"
    case notificationSound = "notification_sound"
    case notificationPreferences = "notification_preferences"
    case attestation
    case lastSeenAt = "last_seen_at"
    case createdAt = "created_at"
    case isCurrent = "is_current"
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    // Identity fields are always present.
    id = try c.decode(String.self, forKey: .id)
    userId = try c.decode(String.self, forKey: .userId)
    name = try c.decode(String.self, forKey: .name)
    platform = try c.decode(NodDevicePlatform.self, forKey: .platform)
    lastSeenAt = try c.decode(Date.self, forKey: .lastSeenAt)
    createdAt = try c.decode(Date.self, forKey: .createdAt)
    // Informational/status fields decode defensively: a single missing one must
    // not reject the whole ClientState (e.g. a runtime build that predates a
    // newly-added field). Defaults are the safe "not set" values.
    nativeAppId = try c.decodeIfPresent(String.self, forKey: .nativeAppId)
    pushProvider = try c.decodeIfPresent(String.self, forKey: .pushProvider)
    hasPushToken = try c.decodeIfPresent(Bool.self, forKey: .hasPushToken) ?? false
    hasSigningKey = try c.decodeIfPresent(Bool.self, forKey: .hasSigningKey) ?? false
    notificationSound = try c.decodeIfPresent(String.self, forKey: .notificationSound) ?? "default"
    notificationPreferences = try c.decodeIfPresent(NodDeviceNotificationPreferences.self, forKey: .notificationPreferences) ?? .init()
    attestation = try c.decodeIfPresent(NodDeviceAttestationSummary.self, forKey: .attestation)
    isCurrent = try c.decodeIfPresent(Bool.self, forKey: .isCurrent) ?? false
  }
}

public enum NodDeviceAttestationStatus: String, Codable, Sendable {
  case verified
  case failed
}

public struct NodDeviceAttestationSummary: Codable, Hashable, Sendable {
  public let provider: String
  public let status: NodDeviceAttestationStatus
  public let keyId: String?
  public let teamId: String?
  public let bundleId: String?
  public let environment: String?
  public let verifiedAt: Date?
  public let failureReason: String?

  enum CodingKeys: String, CodingKey {
    case provider, status, environment
    case keyId = "key_id"
    case teamId = "team_id"
    case bundleId = "bundle_id"
    case verifiedAt = "verified_at"
    case failureReason = "failure_reason"
  }
}

public struct NodNotificationSoundOption: Identifiable, Hashable, Sendable {
  public let id: String
  public let label: String

  public init(id: String, label: String) {
    self.id = id
    self.label = label
  }
}

public enum NodNotificationDeliveryMode: String, Codable, Sendable {
  case push
  case websocket
}

public struct NodNotificationDelivery: Codable, Hashable, Sendable {
  public let mode: NodNotificationDeliveryMode

  public init(mode: NodNotificationDeliveryMode) {
    self.mode = mode
  }
}

public struct NodField: Codable, Hashable, Sendable {
  public let label: String
  public let value: String
  public let style: String?
}

public struct NodLink: Codable, Hashable, Sendable {
  public let label: String
  public let url: String
}

public enum NodOptionKind: String, Codable, Sendable {
  case approve
  case approveWithText = "approve_with_text"
  case reject
  case rejectWithText = "reject_with_text"
  case dismiss
  case open
  case custom

  public init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    let value = try container.decode(String.self)
    self = NodOptionKind(rawValue: value) ?? .custom
  }
}

public struct NodRequestOption: Codable, Identifiable, Hashable, Sendable {
  public let id: String
  public let label: String
  public let kind: NodOptionKind
  public let style: String
  public let requiresText: Bool
  public let textPlaceholder: String?
  public let destructive: Bool
  public let foreground: Bool

  public init(
    id: String,
    label: String,
    kind: NodOptionKind,
    style: String = "default",
    requiresText: Bool = false,
    textPlaceholder: String? = nil,
    destructive: Bool = false,
    foreground: Bool = false
  ) {
    self.id = id
    self.label = label
    self.kind = kind
    self.style = style
    self.requiresText = requiresText
    self.textPlaceholder = textPlaceholder
    self.destructive = destructive
    self.foreground = foreground
  }

  enum CodingKeys: String, CodingKey {
    case id, label, kind, style, destructive, foreground
    case requiresText = "requires_text"
    case textPlaceholder = "text_placeholder"
  }
}

public enum NodRequestStatus: String, Codable, Sendable {
  case pending
  case resolved
  case expired
  case cancelled
}

public struct NodDecision: Codable, Hashable, Sendable {
  public let requestId: String
  public let optionId: String
  public let optionKind: NodOptionKind
  public let optionLabel: String
  public let text: String?
  public let actorUserId: String?
  public let actorDeviceId: String?
  public let signature: NodDecisionSignatureRecord?
  public let resolvedAt: Date

  public init(
    requestId: String,
    optionId: String,
    optionKind: NodOptionKind,
    optionLabel: String,
    text: String? = nil,
    actorUserId: String? = nil,
    actorDeviceId: String? = nil,
    signature: NodDecisionSignatureRecord? = nil,
    resolvedAt: Date
  ) {
    self.requestId = requestId
    self.optionId = optionId
    self.optionKind = optionKind
    self.optionLabel = optionLabel
    self.text = text
    self.actorUserId = actorUserId
    self.actorDeviceId = actorDeviceId
    self.signature = signature
    self.resolvedAt = resolvedAt
  }

  enum CodingKeys: String, CodingKey {
    case text, signature
    case requestId = "request_id"
    case optionId = "option_id"
    case optionKind = "option_kind"
    case optionLabel = "option_label"
    case actorUserId = "actor_user_id"
    case actorDeviceId = "actor_device_id"
    case resolvedAt = "resolved_at"
  }
}

public struct NodDecisionSignatureRecord: Codable, Hashable, Sendable {
  public let keyId: String
  public let algorithm: String
  public let nonce: String
  public let signedAt: String
  public let requestDigest: String
  public let signingPayload: String
  public let signature: String
  public let verified: Bool
  public let publicKey: String?

  enum CodingKeys: String, CodingKey {
    case algorithm, nonce, signature, verified
    case keyId = "key_id"
    case signedAt = "signed_at"
    case requestDigest = "request_digest"
    case signingPayload = "signing_payload"
    case publicKey = "public_key"
  }
}

public struct NodUserDecision: Codable, Hashable, Sendable {
  public let userId: String
  public let decision: NodDecision

  enum CodingKeys: String, CodingKey {
    case decision
    case userId = "user_id"
  }
}

public enum NodDecisionResolution: String, Codable, Sendable {
  case shared
  case perUser = "per_user"
}

public struct NodRequestNotification: Codable, Hashable, Sendable {
  public let redact: Bool
  public let title: String?
  public let body: String?

  enum CodingKeys: String, CodingKey {
    case redact, title, body
  }

  public init(redact: Bool = false, title: String? = nil, body: String? = nil) {
    self.redact = redact
    self.title = title
    self.body = body
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    redact = try container.decodeIfPresent(Bool.self, forKey: .redact) ?? false
    title = try container.decodeIfPresent(String.self, forKey: .title)
    body = try container.decodeIfPresent(String.self, forKey: .body)
  }
}

public struct NodRequestSigning: Codable, Hashable, Sendable {
  public let version: String
  public let recipientsCommitment: String
  public let requestDigest: String

  enum CodingKeys: String, CodingKey {
    case version
    case recipientsCommitment = "recipients_commitment"
    case requestDigest = "request_digest"
  }
}

public struct NodRequest: Codable, Identifiable, Hashable, Sendable {
  public let id: String
  public let requestId: String
  public let channelId: String
  public let recipients: [String]
  public let decisionResolution: NodDecisionResolution
  public let title: String
  public let summary: String
  public let bodyMarkdown: String
  public let fields: [NodField]
  public let links: [NodLink]
  public let imageUrl: String?
  public let notification: NodRequestNotification
  public let dedupeKey: String?
  public let expiresAt: Date?
  public let status: NodRequestStatus
  public let createdAt: Date
  public let updatedAt: Date
  public let resolvedAt: Date?
  public let decision: NodDecision?
  public let decisions: [NodUserDecision]
  public let callbackUrl: String?
  public let options: [NodRequestOption]
  public let requestDigest: String?
  public let signing: NodRequestSigning?

  public init(
    id: String,
    requestId: String,
    channelId: String,
    recipients: [String],
    decisionResolution: NodDecisionResolution,
    title: String,
    summary: String,
    bodyMarkdown: String,
    fields: [NodField],
    links: [NodLink],
    imageUrl: String?,
    notification: NodRequestNotification,
    dedupeKey: String?,
    expiresAt: Date?,
    status: NodRequestStatus,
    createdAt: Date,
    updatedAt: Date,
    resolvedAt: Date?,
    decision: NodDecision?,
    decisions: [NodUserDecision],
    callbackUrl: String?,
    options: [NodRequestOption],
    requestDigest: String?,
    signing: NodRequestSigning? = nil
  ) {
    self.id = id
    self.requestId = requestId
    self.channelId = channelId
    self.recipients = recipients
    self.decisionResolution = decisionResolution
    self.title = title
    self.summary = summary
    self.bodyMarkdown = bodyMarkdown
    self.fields = fields
    self.links = links
    self.imageUrl = imageUrl
    self.notification = notification
    self.dedupeKey = dedupeKey
    self.expiresAt = expiresAt
    self.status = status
    self.createdAt = createdAt
    self.updatedAt = updatedAt
    self.resolvedAt = resolvedAt
    self.decision = decision
    self.decisions = decisions
    self.callbackUrl = callbackUrl
    self.options = options
    self.requestDigest = requestDigest
    self.signing = signing
  }

  enum CodingKeys: String, CodingKey {
    case id, title, summary, fields, links, notification, status, decision, decisions, options, signing
    case requestId = "request_id"
    case channelId = "channel_id"
    case recipients
    case decisionResolution = "decision_resolution"
    case bodyMarkdown = "body_markdown"
    case imageUrl = "image_url"
    case dedupeKey = "dedupe_key"
    case expiresAt = "expires_at"
    case createdAt = "created_at"
    case updatedAt = "updated_at"
    case resolvedAt = "resolved_at"
    case callbackUrl = "callback_url"
    case requestDigest = "request_digest"
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    id = try container.decode(String.self, forKey: .id)
    requestId = try container.decode(String.self, forKey: .requestId)
    channelId = try container.decode(String.self, forKey: .channelId)
    recipients = try container.decodeIfPresent([String].self, forKey: .recipients) ?? []
    decisionResolution = try container.decodeIfPresent(
      NodDecisionResolution.self,
      forKey: .decisionResolution
    ) ?? .shared
    title = try container.decode(String.self, forKey: .title)
    summary = try container.decode(String.self, forKey: .summary)
    bodyMarkdown = try container.decode(String.self, forKey: .bodyMarkdown)
    fields = try container.decodeIfPresent([NodField].self, forKey: .fields) ?? []
    links = try container.decodeIfPresent([NodLink].self, forKey: .links) ?? []
    imageUrl = try container.decodeIfPresent(String.self, forKey: .imageUrl)
    notification = try container.decode(NodRequestNotification.self, forKey: .notification)
    dedupeKey = try container.decodeIfPresent(String.self, forKey: .dedupeKey)
    expiresAt = try container.decodeIfPresent(Date.self, forKey: .expiresAt)
    status = try container.decode(NodRequestStatus.self, forKey: .status)
    createdAt = try container.decode(Date.self, forKey: .createdAt)
    updatedAt = try container.decode(Date.self, forKey: .updatedAt)
    resolvedAt = try container.decodeIfPresent(Date.self, forKey: .resolvedAt)
    decision = try container.decodeIfPresent(NodDecision.self, forKey: .decision)
    decisions = try container.decodeIfPresent([NodUserDecision].self, forKey: .decisions) ?? []
    callbackUrl = try container.decodeIfPresent(String.self, forKey: .callbackUrl)
    options = try container.decode([NodRequestOption].self, forKey: .options)
    requestDigest = try container.decodeIfPresent(String.self, forKey: .requestDigest)
    signing = try container.decodeIfPresent(NodRequestSigning.self, forKey: .signing)
  }
}

public struct NodDeviceSigningKey: Codable, Hashable, Sendable {
  public let keyId: String
  public let algorithm: String
  public let publicKey: String

  public init(keyId: String, algorithm: String, publicKey: String) {
    self.keyId = keyId
    self.algorithm = algorithm
    self.publicKey = publicKey
  }

  enum CodingKeys: String, CodingKey {
    case algorithm
    case keyId = "key_id"
    case publicKey = "public_key"
  }
}

public struct NodDeviceAttestation: Codable, Hashable, Sendable {
  public let provider: String
  public let keyId: String
  public let attestationObject: String

  public init(provider: String, keyId: String, attestationObject: String) {
    self.provider = provider
    self.keyId = keyId
    self.attestationObject = attestationObject
  }

  enum CodingKeys: String, CodingKey {
    case provider
    case keyId = "key_id"
    case attestationObject = "attestation_object"
  }
}

public struct NodAppAttestationRequest: Sendable {
  public let code: String
  public let deviceName: String
  public let platform: NodDevicePlatform
  public let pushProvider: String?
  public let pushToken: String?
  public let signingKey: NodDeviceSigningKey
  public let account: String

  public init(
    code: String,
    deviceName: String,
    platform: NodDevicePlatform,
    pushProvider: String?,
    pushToken: String?,
    signingKey: NodDeviceSigningKey,
    account: String
  ) {
    self.code = code
    self.deviceName = deviceName
    self.platform = platform
    self.pushProvider = pushProvider
    self.pushToken = pushToken
    self.signingKey = signingKey
    self.account = account
  }
}

