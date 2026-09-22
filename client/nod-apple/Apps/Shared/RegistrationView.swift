import NodKit
import SwiftUI

struct RegistrationView: View {
  @Environment(\.dismiss) private var dismiss
  @EnvironmentObject private var store: NodStore
  @State private var setupLink = ""

  var body: some View {
    Form {
      Section("Setup link") {
        TextField("Paste a Nod enrollment link (optional)", text: $setupLink)
          #if os(iOS)
          .textInputAutocapitalization(.never).autocorrectionDisabled(true)
          #endif
        Button("Use Setup Link") {
          if let url = URL(string: setupLink) { store.importEnrollmentLink(url) }
        }.disabled(setupLink.isEmpty || store.isRegistering)
      }
      Section("Server") {
        TextField("Server URL", text: $store.baseURLString)
          #if os(iOS)
          .textInputAutocapitalization(.never)
          .autocorrectionDisabled(true)
          .keyboardType(.URL)
          #endif
      }

      Section("Device") {
        TextField("Device Name", text: $store.deviceName)
          #if os(iOS)
          .textInputAutocapitalization(.words)
          #endif
      }

      Section("Enrollment Code") {
        EnrollmentCodeInput(code: $store.enrollmentCode)
        Text("Get an enrollment code from your Nod server’s admin page. The code connects this device to your account.")
          .font(.caption).foregroundStyle(.secondary)
        Button {
          Task {
            if await store.register() {
              dismiss()
            }
          }
        } label: {
          if store.isRegistering { ProgressView("Registering…") } else { Label("Register Device", systemImage: "person.badge.key") }
        }
        .buttonStyle(.borderedProminent)
        .disabled(
          store.isRegistering || store.enrollmentCode.count < 8 ||
            store.baseURLString.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
            store.deviceName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        )
      }
    }
    .formStyle(.grouped)
    .navigationTitle("Register Device")
    .interactiveDismissDisabled(store.isRegistering)
    .toolbar {
      if store.isRegistered {
        ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(store.isRegistering) }
      }
    }
  }
}

struct EnrollmentCodeInput: View {
  @Binding var code: String

  var body: some View {
    TextField("8-character enrollment code", text: $code)
      .font(.system(.title3, design: .monospaced))
      .textContentType(.oneTimeCode)
      #if os(iOS)
      .textInputAutocapitalization(.characters)
      .autocorrectionDisabled(true)
      .keyboardType(.asciiCapable)
      #endif
      .onChange(of: code) { _, value in
        code = String(value.uppercased().filter { $0.isASCII && ($0.isLetter || $0.isNumber) }.prefix(8))
      }
      .accessibilityLabel("Enrollment code")
  }
}
