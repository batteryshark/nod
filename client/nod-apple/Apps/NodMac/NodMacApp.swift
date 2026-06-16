import AppKit
import NodKit
import SwiftUI

@main
struct NodMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var store = NodStore(
        platform: .macos,
        defaultDeviceName: Host.current().localizedName ?? "Mac",
        presentLocalNotifications: true
    )

    var body: some Scene {
        WindowGroup {
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
                NSApp.activate(ignoringOtherApps: true)
                if NSApp.windows.isEmpty {
                    NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
                }
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
                    updateDockBadge()
                }
                .onChange(of: store.totalPendingCount) {
                    updateDockBadge()
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

    private func updateDockBadge() {
        NSApp.dockTile.badgeLabel = store.totalPendingCount > 0 ? "\(store.totalPendingCount)" : nil
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
    // NSImage is not Sendable; the only consumer is the main-actor view body.
    @MainActor static let image: NSImage = {
        let image = NSImage(size: NSSize(width: 18, height: 18))
        image.lockFocus()

        NSColor.black.setFill()
        let headPath = NSBezierPath()
        headPath.move(to: CGPoint(x: 2.2, y: 7.2))
        headPath.curve(
            to: CGPoint(x: 8.8, y: 15.0),
            controlPoint1: CGPoint(x: 1.7, y: 12.0),
            controlPoint2: CGPoint(x: 4.6, y: 15.5)
        )
        headPath.curve(
            to: CGPoint(x: 16.4, y: 7.8),
            controlPoint1: CGPoint(x: 13.1, y: 14.5),
            controlPoint2: CGPoint(x: 17.0, y: 12.3)
        )
        headPath.curve(
            to: CGPoint(x: 12.5, y: 1.7),
            controlPoint1: CGPoint(x: 15.8, y: 3.9),
            controlPoint2: CGPoint(x: 14.4, y: 2.1)
        )
        headPath.curve(
            to: CGPoint(x: 5.0, y: 2.3),
            controlPoint1: CGPoint(x: 9.0, y: 1.1),
            controlPoint2: CGPoint(x: 6.2, y: 1.4)
        )
        headPath.curve(
            to: CGPoint(x: 2.2, y: 7.2),
            controlPoint1: CGPoint(x: 3.0, y: 3.7),
            controlPoint2: CGPoint(x: 2.3, y: 5.4)
        )
        headPath.close()
        headPath.fill()

        NSGraphicsContext.current?.compositingOperation = .clear
        NSBezierPath(roundedRect: NSRect(x: 6.8, y: 6.0, width: 2.0, height: 4.2), xRadius: 1.0, yRadius: 1.0).fill()
        NSBezierPath(roundedRect: NSRect(x: 11.6, y: 5.5, width: 2.0, height: 4.2), xRadius: 1.0, yRadius: 1.0).fill()
        NSGraphicsContext.current?.compositingOperation = .sourceOver

        image.unlockFocus()
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
