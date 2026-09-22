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

    private var openHandler: (@MainActor (NodNotificationTarget) -> Void)?
    private var localCategories: [String: UNNotificationCategory] = [:]
    private var optionHandler: (@Sendable (NodNotificationTarget, String, String?) async -> Bool)?

    public func configure(
        onOpen: @escaping @MainActor (NodNotificationTarget) -> Void = { _ in },
        onOption: @escaping @Sendable (NodNotificationTarget, String, String?) async -> Bool = { _, _, _ in false }
    ) {
        self.openHandler = onOpen
        self.optionHandler = onOption
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
        let approve = UNNotificationAction(identifier: "approve", title: "Approve", options: [.authenticationRequired])
        let reject = UNNotificationAction(identifier: "reject", title: "Reject", options: [.destructive, .authenticationRequired])
        let approveNotes = UNTextInputNotificationAction(
            identifier: "approve_notes",
            title: "Approve with Notes",
            options: [.authenticationRequired],
            textInputButtonTitle: "Approve",
            textInputPlaceholder: "Notes"
        )
        let rejectReason = UNTextInputNotificationAction(
            identifier: "reject_reason",
            title: "Reject with Reason",
            options: [.destructive, .authenticationRequired],
            textInputButtonTitle: "Reject",
            textInputPlaceholder: "Reason"
        )
        let open = UNNotificationAction(identifier: "open", title: "Open", options: [.foreground])

        let defaultCategory = UNNotificationCategory(
            identifier: "NOD_DEFAULT",
            actions: [open],
            intentIdentifiers: []
        )
        let approvalCategory = UNNotificationCategory(
            identifier: "NOD_APPROVAL",
            actions: [approve, reject, open],
            intentIdentifiers: []
        )
        let approvalTextCategory = UNNotificationCategory(
            identifier: "NOD_APPROVAL_TEXT",
            actions: [approve, approveNotes, reject, rejectReason, open],
            intentIdentifiers: []
        )
        UNUserNotificationCenter.current().setNotificationCategories(Set([defaultCategory, approvalCategory, approvalTextCategory] + Array(localCategories.values)))
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
        content.categoryIdentifier = hidePreviews || request.notification.redact ? "NOD_DEFAULT" : localCategory(for: request, target: target)
        content.userInfo = ["request_id": request.id, "channel_id": request.channelId]
        if let serverId = target.serverId { content.userInfo["server_id"] = serverId }
        // Alerts must not wait on issuer-controlled media or expose images hidden by preview policy.
        try await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: target.notificationId, content: content, trigger: nil))
    }

    private func localCategory(for request: NodRequest, target: NodNotificationTarget) -> String {
        let actions = Self.actions(for: request)
        guard actions.count > 1 else { return "NOD_DEFAULT" }
        let identifier = "NOD_LOCAL_" + target.notificationId
        // Keep categories for delivered alerts. A missing older category safely falls back to opening the app.
        if localCategories.count >= 64, let expiredCategory = localCategories.keys.sorted().first { localCategories.removeValue(forKey: expiredCategory) }
        localCategories[identifier] = UNNotificationCategory(identifier: identifier, actions: actions, intentIdentifiers: [])
        registerCategories()
        return identifier
    }

    nonisolated static func actions(for request: NodRequest) -> [UNNotificationAction] {
        let open = UNNotificationAction(identifier: "open", title: "Open Nod", options: [.foreground])
        guard !request.notification.redact, !request.options.isEmpty, request.options.count <= 3 else { return [open] }
        let actions: [UNNotificationAction] = request.options.map { option in
            var flags: UNNotificationActionOptions = [.authenticationRequired]
            if option.destructive { flags.insert(.destructive) }
            if option.foreground || option.requiresText { flags.insert(.foreground) }
            let actionId = "nod.option." + option.id
            if option.requiresText {
                return UNTextInputNotificationAction(identifier: actionId, title: option.label, options: flags, textInputButtonTitle: "Submit", textInputPlaceholder: option.textPlaceholder ?? "Notes")
            }
            return UNNotificationAction(identifier: actionId, title: option.label, options: flags)
        }
        return actions + [open]
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
        let target = NodNotificationTarget.decode(response.notification.request.content.userInfo)
        if response.actionIdentifier == UNNotificationDefaultActionIdentifier || response.actionIdentifier == "open" {
            await MainActor.run { self.openHandler?(target) }
            return
        }
        guard response.actionIdentifier != UNNotificationDismissActionIdentifier, target.requestId != nil else { return }
        let prefix = "nod.option."
        let optionId = response.actionIdentifier.hasPrefix(prefix) ? String(response.actionIdentifier.dropFirst(prefix.count)) : response.actionIdentifier
        let text = (response as? UNTextInputNotificationResponse)?.userText
        let handler = await MainActor.run { self.optionHandler }
        if await handler?(target, optionId, text) != true {
            let content = UNMutableNotificationContent()
            content.title = "Nod response was not sent"
            content.body = "Open Nod to review the request and retry."
            content.userInfo = response.notification.request.content.userInfo
            content.categoryIdentifier = "NOD_DEFAULT"
            try? await center.add(UNNotificationRequest(identifier: response.notification.request.identifier, content: content, trigger: nil))
        }
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
