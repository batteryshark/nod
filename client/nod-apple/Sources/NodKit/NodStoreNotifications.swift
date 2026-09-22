import Foundation

extension NodStore {
  public func requestNotifications(reportMissingGrant: Bool = true) async {
    if let issue = notificationRuntimeIssue() {
      notificationPermissionIssue = issue
      return
    }

    do {
      let authorized = try await NodNotificationController.shared.requestAuthorization()
      await updateNotificationPermissionIssue(
        authorized: authorized,
        reportMissingGrant: reportMissingGrant
      )
    } catch {
      notificationPermissionIssue = "Could not request notification permission: \(error.localizedDescription)"
      lastError = error.localizedDescription
    }
  }

  public func refreshNotificationAuthorizationStatus() async {
    let settings = await NodNotificationController.shared.notificationSettings()
    notificationAuthorizationStatus = settings.authorizationStatus
  }

  public func requestAndTestNotifications() async {
    await requestNotifications()
    guard notificationPermissionIssue == nil else {
      return
    }

    do {
      try await NodNotificationController.shared.presentTestNotification(
        soundName: notificationSound
      )
    } catch {
      let settings = await NodNotificationController.shared.notificationSettings()
      notificationPermissionIssue =
        notificationPermissionIssue(for: settings, reportMissingGrant: true)
        ?? "Could not show Nod test notification: \(error.localizedDescription)"
    }
  }

  public func openNotification(_ target: NodNotificationTarget) async {
    do {
      try await ensureRuntimeStarted()
      let profiles = runtime.state?.servers.map { NodServerProfile(id: $0.id, name: $0.name, baseURLString: $0.baseUrlString, deviceName: $0.deviceName, deviceId: $0.deviceId) } ?? servers
      guard let serverId = NodNotificationPolicy.serverId(for: target, servers: profiles), let requestId = target.requestId else {
        throw NodStoreError.ambiguousNotificationServer
      }
      try await runtime.openRequest(serverId: serverId, requestId: requestId)
      // Apply selection before publishing navigation, without waiting for Combine delivery.
      selectedChannelId = runtime.state?.selectedChannelId ?? target.channelId
      selectedRequestId = requestId
      notificationOpenRequest = NodNotificationOpenRequest(requestId: requestId, channelId: selectedChannelId, serverId: serverId)
      connectSync()
    } catch { mapRuntimeError(error) }
  }

  func presentNotificationCandidates(_ candidates: [NodNotificationCandidate]) async {
    guard shouldPresentLocalNotificationFromSync() else { return }
    for candidate in candidates {
      let key = candidate.target.notificationId
      guard !presentedNotificationRequestIds.contains(key) else { continue }
      do {
        try await NodNotificationController.shared.presentLocalNotification(for: candidate, soundName: notificationSound)
        presentedNotificationRequestIds.insert(key)
      } catch {
        notificationPermissionIssue = "Could not show Nod notification: \(error.localizedDescription)"
      }
    }
  }

  func shouldPresentLocalNotificationFromSync() -> Bool {
    NodNotificationPolicy.shouldPresentLocalNotification(
      presentLocalNotifications: presentLocalNotifications,
      deliveryMode: notificationDeliveryMode
    )
  }

  private func updateNotificationPermissionIssue(authorized: Bool, reportMissingGrant: Bool) async {
    let settings = await NodNotificationController.shared.notificationSettings()
    notificationAuthorizationStatus = settings.authorizationStatus
    notificationPermissionIssue = notificationPermissionIssue(
      for: settings,
      reportMissingGrant: reportMissingGrant && !authorized
    )
  }

  private func notificationRuntimeIssue() -> String? {
    guard shouldPresentLocalNotificationFromSync() else {
      return nil
    }

    #if os(macOS)
      if Bundle.main.bundleURL.pathExtension != "app" {
        return
          "Nod is running outside the Nod.app bundle, so macOS cannot add it to Notification Settings. Launch the built Nod.app instead of the SwiftPM executable."
      }
    #endif

    return nil
  }

  private func notificationPermissionIssue(
    for settings: NodNotificationSettings,
    reportMissingGrant: Bool
  ) -> String? {
    switch settings.authorizationStatus {
    case .denied:
      return
        "Notifications are disabled for Nod. Enable them in Settings > Notifications to receive alerts."
    case .notDetermined:
      if reportMissingGrant {
        return "Nod has not been granted notification permission yet."
      }
    case .authorized, .provisional, .ephemeral, .unknown:
      break
    }

    if settings.alertSetting == .disabled {
      return
        "Nod notifications are allowed, but alert banners are disabled. Enable banners for Nod in Settings > Notifications."
    }

    return nil
  }
}
