import NodKit
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import ServiceManagement
#endif

struct SubscriptionSheet: View {
  @Environment(\.dismiss) private var dismiss
  @EnvironmentObject private var store: NodStore
  @AppStorage("nod.loadRemoteMedia") private var loadRemoteMedia = false
  #if os(macOS)
  @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
  @State private var preferenceError: String?
  #endif

  var body: some View {
    NavigationStack {
      Form {
        Section("Notification Sound") {
          Picker("Sound", selection: Binding(
            get: { store.notificationSound },
            set: { sound in
              Task {
                await store.setNotificationSound(sound)
              }
            }
          )) {
            ForEach(NodStore.notificationSoundOptions) { option in
              Text(option.label).tag(option.id)
            }
          }
          #if os(macOS)
          Button {
            Task { await store.requestAndTestNotifications() }
          } label: {
            Label("Request/Test Notifications", systemImage: "bell.badge")
          }
          Button {
            openNodNotificationSettings()
          } label: {
            Label("Open Notification Settings", systemImage: "gear")
          }
          #endif
        }

        deviceAlertPreferences

        Section("Reading") {
          Toggle("Acknowledge informational requests on open", isOn: $store.acknowledgeOnOpen)
          Text("Acknowledging completes a request. Shared requests are completed for everyone; turn this off to acknowledge manually.")
            .font(.caption).foregroundStyle(.secondary)
        }
        Section("Privacy") {
          Toggle("Load remote images and link previews", isOn: $loadRemoteMedia)
          Text("Remote media can contact the sites included in requests. You can always open a link explicitly.")
            .font(.caption).foregroundStyle(.secondary)
        }
        #if os(iOS)
        Section("Notifications") {
          if store.notificationDeliveryMode == .websocket {
            Text("This server currently uses foreground sync. Background push is not available for this device.")
              .font(.caption).foregroundStyle(.secondary)
          }
          Button("Test Notification on This Device") { Task { await store.requestAndTestNotifications() } }
          Button("Open Notification Settings") {
            if let url = URL(string: UIApplication.openNotificationSettingsURLString) { UIApplication.shared.open(url) }
          }
          if let issue = store.notificationPermissionIssue { Text(issue).foregroundStyle(.secondary) }
        }
        #elseif os(macOS)
        Section("Startup") {
          Toggle("Launch Nod at login", isOn: Binding(get: { launchAtLogin }, set: { enabled in
            do {
              if enabled { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
              launchAtLogin = SMAppService.mainApp.status == .enabled
              preferenceError = SMAppService.mainApp.status == .requiresApproval ? "Allow Nod in System Settings > General > Login Items." : nil
            } catch { preferenceError = error.localizedDescription }
          }))
          if let preferenceError { Text(preferenceError).foregroundStyle(.red) }
        }
        #endif
        Section("Channels") {
          ForEach(store.channels) { channel in
            Toggle(isOn: Binding(
              get: { store.channels.first(where: { $0.id == channel.id })?.subscribed ?? false },
              set: { subscribed in
                Task {
                  await store.setSubscription(channelId: channel.id, subscribed: subscribed)
                }
              }
            )) {
              ChannelLabel(channel: channel)
            }
          }
        }
      }
      .navigationTitle("Settings")
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done") {
            dismiss()
          }
        }
      }
    }
  }

  private var deviceAlertPreferences: some View {
    Section("Alerts on This Device") {
      Text("Preferences for " + (store.selectedServer?.name ?? "this server"))
        .font(.caption).foregroundStyle(.secondary)
      Toggle("Hide notification content", isOn: Binding(
        get: { store.deviceNotificationPreferences.hideContent },
        set: { hidden in updateDevicePreferences { $0.hideContent = hidden } }
      ))
      Menu("Pause notifications") {
        ForEach([1, 8, 24], id: \.self) { hours in
          Button("For \(hours) \(hours == 1 ? "hour" : "hours")") {
            updateDevicePreferences { $0.snoozedUntil = Date().addingTimeInterval(Double(hours) * 3600) }
          }
        }
      }
      if let until = store.deviceNotificationPreferences.snoozedUntil, until > Date() {
        Text("Paused until ") + Text(until, format: .dateTime)
        Button("Resume notifications now") { updateDevicePreferences { $0.snoozedUntil = nil } }
      }
      ForEach(store.channels) { channel in
        Toggle("Mute " + channel.name, isOn: Binding(
          get: { store.deviceNotificationPreferences.mutedChannels.contains(channel.id) },
          set: { muted in
            updateDevicePreferences { preferences in
              preferences.mutedChannels.removeAll { $0 == channel.id }
              if muted { preferences.mutedChannels.append(channel.id) }
            }
          }
        ))
      }
      Text("Muted and paused requests stay in your inbox. Use Focus in system settings for scheduled quiet hours.")
        .font(.caption).foregroundStyle(.secondary)
      if store.isUpdatingNotificationPreferences { ProgressView("Saving notification preferences…") }
      if let error = store.lastError { Text(error).font(.caption).foregroundStyle(.red) }
    }
    .disabled(store.isUpdatingNotificationPreferences || store.registeredDevices.isEmpty)
  }

  private func updateDevicePreferences(_ change: (inout NodDeviceNotificationPreferences) -> Void) {
    guard let serverId = store.selectedServerId else { return }
    var preferences = store.deviceNotificationPreferences
    change(&preferences)
    Task { await store.setDeviceNotificationPreferences(preferences, serverId: serverId) }
  }
}
