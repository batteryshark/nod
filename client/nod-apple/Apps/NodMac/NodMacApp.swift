import AppKit
import Combine
import NodKit
import SwiftUI

@main
struct NodMacApp: App {
    @Environment(\.openWindow) private var openWindow
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var store: NodStore

    init() {
        let store = NodStore(
            platform: .macos,
            defaultDeviceName: Host.current().localizedName ?? "Mac",
            presentLocalNotifications: true
        )
        _store = StateObject(wrappedValue: store)
        // MenuBarExtra labels (and hidden windows) need not run view tasks.
        // Bind once at app construction, before any foreground transition.
        appDelegate.observeStore(store)
    }

    var body: some Scene {
        Window("Nod", id: "inbox") {
            NodRootView()
                .environmentObject(store)
                .frame(minWidth: 860, minHeight: 560)
        }
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Refresh") {
                    Task { await store.refresh() }
                }
                .keyboardShortcut("r")
            }
        }

        MenuBarExtra {
            if store.totalPendingCount > 0 {
                Text("\(store.totalPendingCount) pending")
                Divider()
            }
            Button("Open Nod") {
                openWindow(id: "inbox")
                NSApp.activate(ignoringOtherApps: true)
            }
            Button("Refresh") {
                Task { await store.refresh() }
            }
            Divider()
            Text("Notifications: \(notificationStatusLabel)")
            Button("Request Notification Permission") {
                Task { await store.requestNotifications() }
            }
            Button("Open Notification Settings") {
                openNotificationSettings()
            }
            Divider()
            Button("Quit") {
                NSApp.terminate(nil)
            }
        } label: {
            Image(nsImage: NodMenuBarIcon.image)
                .accessibilityLabel(menuBarTitle)
                .task {
                    await store.refreshNotificationAuthorizationStatus()
                }
                .onChange(of: store.notificationOpenRequest) {
                    openWindow(id: "inbox")
                    NSApp.activate(ignoringOtherApps: true)
                }
        }
    }

    private var notificationStatusLabel: String {
        switch store.notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral:
            return "Granted"
        case .denied:
            return "Denied"
        case .notDetermined:
            return "Not Determined"
        case .unknown:
            return "Unknown"
        }
    }

    private var menuBarTitle: String {
        guard store.totalPendingCount > 0 else {
            return "Nod"
        }
        return "Nod \(store.totalPendingCount)"
    }

    private func openNotificationSettings() {
        let urls = [
            "x-apple.systempreferences:com.apple.preference.notifications?id=com.batteryshark.NodMac",
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        ]
        for urlString in urls {
            guard let url = URL(string: urlString), NSWorkspace.shared.open(url) else {
                continue
            }
            return
        }
    }
}

private enum NodMenuBarIcon {
    /// The menu-bar glyph is the Nod mascot line art, shipped as a template
    /// image set in the shared asset catalog: an 18 pt bitmap (18px @1x,
    /// 36px @2x) whose shape lives in alpha, so macOS tints it for light and
    /// dark menu bars and for the selected (inverted) state.
    ///
    /// Asset catalog wiring matches how the app icons already ship: the mac
    /// and iOS targets both compile `Shared/Assets.xcassets`. The bare SwiftPM
    /// executable excludes the catalog (see `Package.swift`), so a missing
    /// image is reported instead of crashing; it is only used as a compile check.
    // NSImage is not Sendable; the only consumer is the main-actor view body.
    @MainActor static let image: NSImage = {
        guard let image = NSImage(named: "NodMenuBarIcon") else {
            NSLog("Nod menu bar icon missing: NodMenuBarIcon is not in the asset catalog")
            return NSImage(size: NSSize(width: 18, height: 18))
        }
        image.size = NSSize(width: 18, height: 18)
        image.isTemplate = true
        return image
    }()
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Held for the app's lifetime to keep macOS App Nap from throttling the
    /// process while Nod lives in the menu bar with no focused window. App Nap
    /// suspends the background process — including the runtime's WebSocket sync
    /// thread — so new requests would only surface after the app was refocused
    /// (which forces a reconnect + refresh). `userInitiatedAllowingIdleSystemSleep`
    /// keeps the sync socket serviced in the background while still letting the
    /// Mac sleep normally on battery.
    private var syncActivityAssertion: NSObjectProtocol?
    private var badgeSubscription: AnyCancellable?
    private var foregroundSubscription: AnyCancellable?

    @MainActor
    func observeStore(_ store: NodStore) {
        guard badgeSubscription == nil else { return }
        // @Published delivers the current counts immediately, then every
        // snapshot/live change. Keep observing even when no view redraws.
        badgeSubscription = store.$pendingCountsByChannel
            .map { $0.values.reduce(0, +) }
            .sink { count in
                NSApplication.shared.dockTile.badgeLabel = count > 0 ? "\(count)" : nil
                NSLog("Nod pending badge updated: %ld", count)
            }
        foregroundSubscription = NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
            .sink { [weak store] _ in
                Task { @MainActor in
                    guard let store else { return }
                    let previous = store.lastSyncedAt
                    await store.resumeFromForeground()
                    NSLog("Nod foreground refresh finished: refreshed=%@ phase=%@ pending=%ld",
                          store.lastSyncedAt != previous ? "yes" : "no", store.syncPhase, store.totalPendingCount)
                }
            }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        syncActivityAssertion = ProcessInfo.processInfo.beginActivity(
            options: .userInitiatedAllowingIdleSystemSleep,
            reason: "Maintain Nod realtime sync for notifications"
        )
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let assertion = syncActivityAssertion {
            ProcessInfo.processInfo.endActivity(assertion)
            syncActivityAssertion = nil
        }
    }
}
