import Foundation
import NodClientFFI

public struct NodNotificationTarget: Equatable, Sendable {
  public let serverId: String?
  public let deviceId: String?
  public let requestId: String?
  public let channelId: String?

  public init(serverId: String? = nil, deviceId: String? = nil, requestId: String? = nil, channelId: String? = nil) {
    self.serverId = serverId
    self.deviceId = deviceId
    self.requestId = requestId
    self.channelId = channelId
  }

  public var notificationId: String {
    [serverId, requestId].compactMap { $0 }.joined(separator: ":")
  }

  func matches(_ notification: NodNotificationTarget) -> Bool {
    (requestId == nil || notification.requestId == requestId)
      && (channelId == nil || notification.channelId == channelId)
      && (serverId == nil || notification.serverId == serverId || (deviceId != nil && notification.deviceId == deviceId))
  }

  public static func decode(_ userInfo: [AnyHashable: Any]) -> Self {
    let metadata = userInfo["nod"] as? [AnyHashable: Any] ?? userInfo
    func string(_ key: String) -> String? {
      guard let value = metadata[key] as? String, !value.isEmpty else { return nil }
      return value
    }
    return Self(serverId: string("server_id"), deviceId: string("device_id"), requestId: string("request_id"), channelId: string("channel_id"))
  }
}

public struct NodNotificationCandidate: Sendable {
  public let serverId: String?
  public let request: NodRequest

  public var target: NodNotificationTarget {
    NodNotificationTarget(serverId: serverId, requestId: request.id, channelId: request.channelId)
  }
}

public enum NodNotificationPolicy {
  public static func shouldPresentLocalNotification(presentLocalNotifications: Bool, deliveryMode: NodNotificationDeliveryMode) -> Bool {
    presentLocalNotifications || deliveryMode == .websocket
  }

  public static func preview(for request: NodRequest, hidePreviews: Bool = false) -> (title: String, body: String) {
    if hidePreviews { return ("Nod", "Open Nod to review a request.") }
    struct Preview: Decodable { let title: String; let body: String }
    // Fail closed if the wire contract cannot be rendered by the shared policy.
    guard let encoded = try? JSONEncoder.nod.encode(request),
          let json = try? notificationPreview(requestJson: String(decoding: encoded, as: UTF8.self)),
          let preview = try? JSONDecoder().decode(Preview.self, from: Data(json.utf8)) else {
      return ("Nod", "Open Nod to review a request.")
    }
    return (preview.title, preview.body)
  }

  /// Older pushes lack server identity. Resolve only when there is no ambiguity.
  public static func serverId(for target: NodNotificationTarget, servers: [NodServerProfile]) -> String? {
    if let serverId = target.serverId { return servers.first { $0.id == serverId }?.id }
    if let deviceId = target.deviceId { return servers.first { $0.deviceId == deviceId }?.id }
    return servers.count == 1 ? servers.first?.id : nil
  }
}
