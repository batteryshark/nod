import Foundation
import UserNotifications

public struct NodNotificationSettings: Sendable {
    public let authorizationStatus: NodNotificationAuthorizationStatus
    public let alertSetting: NodNotificationAlertSetting

    fileprivate init(_ settings: UNNotificationSettings) {
        self.authorizationStatus = NodNotificationAuthorizationStatus(settings.authorizationStatus)
        self.alertSetting = NodNotificationAlertSetting(settings.alertSetting)
    }
}

public enum NodNotificationAuthorizationStatus: Sendable {
    case notDetermined
    case denied
    case authorized
    case provisional
    case ephemeral
    case unknown

    fileprivate init(_ status: UNAuthorizationStatus) {
        switch status {
        case .notDetermined:
            self = .notDetermined
        case .denied:
            self = .denied
        case .authorized:
            self = .authorized
        case .provisional:
            self = .provisional
        case .ephemeral:
            self = .ephemeral
        @unknown default:
            self = .unknown
        }
    }
}

public enum NodNotificationAlertSetting: Sendable {
    case notSupported
    case disabled
    case enabled
    case unknown

    fileprivate init(_ setting: UNNotificationSetting) {
        switch setting {
        case .notSupported:
            self = .notSupported
        case .disabled:
            self = .disabled
        case .enabled:
            self = .enabled
        @unknown default:
            self = .unknown
        }
    }
}

@MainActor
public final class NodNotificationController: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = NodNotificationController()

    /// Category identifiers a server push may carry. Every one offers only the
    /// foreground `open` action: an alert's text is relay-controlled and never
    /// shows verified request content, so no decision is signed from a notification.
    nonisolated public static let categoryIdentifiers = ["NOD_DEFAULT", "NOD_APPROVAL", "NOD_APPROVAL_TEXT"]

    private var openHandler: (@MainActor (NodNotificationTarget) -> Void)?

    public func configure(onOpen: @escaping @MainActor (NodNotificationTarget) -> Void = { _ in }) {
        self.openHandler = onOpen
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        registerCategories()
    }

    public func requestAuthorization() async throws -> Bool {
        try await UNUserNotificationCenter.current().requestAuthorization(
            options: [.alert, .badge, .sound, .providesAppNotificationSettings]
        )
    }

    public func notificationSettings() async -> NodNotificationSettings {
        await withCheckedContinuation { continuation in
            UNUserNotificationCenter.current().getNotificationSettings { settings in
                continuation.resume(returning: NodNotificationSettings(settings))
            }
        }
    }

    public func registerCategories() {
        UNUserNotificationCenter.current().setNotificationCategories(Set(Self.categories()))
    }

    /// Older servers still push `NOD_APPROVAL`/`NOD_APPROVAL_TEXT`; keep those
    /// identifiers registered so the alert shows, but with the open action only.
    nonisolated static func categories() -> [UNNotificationCategory] {
        categoryIdentifiers.map { identifier in
            UNNotificationCategory(identifier: identifier, actions: [openAction()], intentIdentifiers: [])
        }
    }

    nonisolated static func openAction() -> UNNotificationAction {
        UNNotificationAction(identifier: "open", title: "Open Nod", options: [.foreground])
    }

    public func presentLocalNotification(for candidate: NodNotificationCandidate, soundName: String, hidePreviews: Bool = false) async throws {
        let request = candidate.request
        let target = candidate.target
        let content = UNMutableNotificationContent()
        let preview = NodNotificationPolicy.preview(for: request, hidePreviews: hidePreviews)
        content.title = preview.title
        content.body = preview.body
        content.sound = notificationSound(named: soundName)
        content.threadIdentifier = [target.serverId, request.channelId].compactMap { $0 }.joined(separator: ":")
        content.categoryIdentifier = "NOD_DEFAULT"
        content.userInfo = ["request_id": request.id, "channel_id": request.channelId]
        if let serverId = target.serverId { content.userInfo["server_id"] = serverId }
        // Alerts must not wait on issuer-controlled media or expose images hidden by preview policy.
        try await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: target.notificationId, content: content, trigger: nil))
    }

    public func removeNotifications(for target: NodNotificationTarget) async {
        await removeNotifications(for: [target])
    }

    public func removeNotifications(for targets: [NodNotificationTarget]) async {
        let center = UNUserNotificationCenter.current()
        let delivered = await center.deliveredNotifications()
        let pending = await center.pendingNotificationRequests()
        func matches(_ request: UNNotificationRequest) -> Bool {
            let context = NodNotificationTarget.decode(request.content.userInfo)
            return targets.contains { $0.matches(context) }
        }
        center.removeDeliveredNotifications(withIdentifiers: delivered.map(\.request).filter(matches).map(\.identifier))
        center.removePendingNotificationRequests(withIdentifiers: pending.filter(matches).map(\.identifier))
    }

    nonisolated public func presentTestNotification(soundName: String) async throws {
        let content = UNMutableNotificationContent()
        content.title = "Nod notifications are on"
        content.body = "Nod can show alerts on this device. This test does not verify server push delivery."
        content.sound = notificationSound(named: soundName)
        let request = UNNotificationRequest(
            identifier: "nod.notification-test.\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        try await UNUserNotificationCenter.current().add(request)
    }

    nonisolated public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    nonisolated public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        // Any action other than dismiss only opens the request in the app. An
        // alert delivered under an older category may still name an approve or
        // reject action; those also open the card and never submit.
        guard response.actionIdentifier != UNNotificationDismissActionIdentifier else { return }
        let target = NodNotificationTarget.decode(response.notification.request.content.userInfo)
        await MainActor.run { self.openHandler?(target) }
    }

    nonisolated private func notificationSound(named sound: String?) -> UNNotificationSound? {
        guard let sound = sound?.trimmingCharacters(in: .whitespacesAndNewlines), !sound.isEmpty else {
            return .default
        }
        if sound == "none" || sound == "silent" {
            return nil
        }
        if sound == "default" {
            return .default
        }
        return UNNotificationSound(named: UNNotificationSoundName(sound))
    }
}
