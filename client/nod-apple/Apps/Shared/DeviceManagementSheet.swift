import NodKit
import SwiftUI

struct DeviceManagementSheet: View {
  @Environment(\.dismiss) private var dismiss
  @EnvironmentObject private var store: NodStore
  @State private var renamingDevice: NodUserDevice?
  @State private var renameText = ""
  @State private var revokingDevice: NodUserDevice?

  var body: some View {
    NavigationStack {
      Form {
        Section("Account") {
          LabeledContent("User", value: accountName)
          if let server = store.selectedServer {
            LabeledContent("Server", value: server.name)
          }
        }

        Section("Devices") {
          if store.registeredDevices.isEmpty {
            ContentUnavailableView(
              "No Registered Devices",
              systemImage: "desktopcomputer.trianglebadge.exclamationmark"
            )
          } else {
            ForEach(store.registeredDevices) { device in
              DeviceRow(
                device: device,
                rename: {
                  renameText = device.name
                  renamingDevice = device
                },
                revoke: {
                  revokingDevice = device
                }
              )
            }
          }
        }
      }
      .formStyle(.grouped)
      .navigationTitle("Devices")
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done") {
            dismiss()
          }
        }
      }
      .task {
        await store.refreshAccount()
      }
      .sheet(item: $renamingDevice) { device in
        renameSheet(for: device)
      }
      .confirmationDialog(
        "Revoke Device",
        isPresented: Binding(
          get: { revokingDevice != nil },
          set: { if !$0 { revokingDevice = nil } }
        ),
        presenting: revokingDevice
      ) { device in
        Button(
          device.isCurrent ? "Revoke From This Server" : "Revoke Device",
          role: .destructive
        ) {
          Task {
            let succeeded = await store.revokeDevice(device)
            if succeeded && device.isCurrent {
              dismiss()
            }
            revokingDevice = nil
          }
        }
      } message: { device in
        Text(revokeConfirmationMessage(for: device))
      }
    }
  }

  private var accountName: String {
    if let user = store.currentUser {
      return "\(user.name) (\(user.id))"
    }
    if let server = store.selectedServer, let userName = server.userName, let userId = server.userId {
      return "\(userName) (\(userId))"
    }
    return "Unknown"
  }

  private func renameSheet(for device: NodUserDevice) -> some View {
    NavigationStack {
      Form {
        TextField("Device Name", text: $renameText)
        if let error = store.lastError { Text(error).foregroundStyle(.red) }
      }
      .navigationTitle("Rename Device")
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel") {
            renamingDevice = nil
          }
        }
        ToolbarItem(placement: .confirmationAction) {
          Button("Save") {
            let name = renameText
            Task {
              if await store.renameDevice(device, name: name) { renamingDevice = nil }
            }
          }
          .disabled(store.pendingActions.contains("rename:" + device.id) || renameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
      }
    }
  }

  private func revokeConfirmationMessage(for device: NodUserDevice) -> String {
    guard device.isCurrent else {
      return "This removes the device from the server."
    }
    if let server = store.selectedServer {
      return "This removes this device from \(server.name) and signs it out locally from that server."
    }
    return "This removes this device from the selected server and signs it out locally from that server."
  }
}

private struct DeviceRow: View {
  let device: NodUserDevice
  let rename: () -> Void
  let revoke: () -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      HStack(alignment: .firstTextBaseline) {
        VStack(alignment: .leading, spacing: 2) {
          Text(device.name)
            .font(.headline)
          Text("\(platformLabel(device.platform)) • Last seen \(device.lastSeenAt.formatted(date: .abbreviated, time: .shortened))")
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        Spacer()
        if device.isCurrent {
          Text("This Device")
            .font(.caption)
            .foregroundStyle(.secondary)
        }
      }

      HStack {
        Button {
          rename()
        } label: {
          Label("Rename", systemImage: "pencil")
        }
        Button(role: .destructive) {
          revoke()
        } label: {
          Label("Revoke", systemImage: "xmark.circle")
        }
      }
      .buttonStyle(.borderless)
    }
    .padding(.vertical, 4)
  }

  private func platformLabel(_ platform: NodDevicePlatform) -> String {
    switch platform {
    case .ios:
      return "iOS"
    case .macos:
      return "macOS"
    case .watchos:
      return "watchOS"
    case .windows:
      return "Windows"
    case .linux:
      return "Linux"
    case .unknown:
      return "Unknown"
    }
  }
}
