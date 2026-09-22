import Foundation
import NodClientFFI

/// Server-address helpers. The implementations live in `nod-client-core` (Rust)
/// and are reached through `NodClientFFI`, so the Apple apps, the TUI, and the
/// desktop all share one canonical normalization/profile-id/display-name logic
/// instead of re-deriving it per platform. A parity test in `nod-client-ffi`
/// (`matches_nodkit_server_address_vectors`) pins these to the original Swift
/// outputs.
public enum NodServerAddress {
  public static func normalizedBaseURL(_ value: String) -> String {
    NodClientFFI.normalizeBaseUrl(value: value)
  }

  public static func profileId(for baseURLString: String) -> String {
    NodClientFFI.profileIdFor(baseUrl: baseURLString)
  }

  public static func displayName(for baseURLString: String) -> String {
    NodClientFFI.displayNameFor(baseUrl: baseURLString)
  }
}

public struct NodEnrollmentLink: Equatable, Sendable {
  public let serverURL: String
  public let code: String
  public let deviceName: String?

  public init(url: URL) throws {
    guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
          components.scheme?.lowercased() == "nod", components.host == "enroll",
          components.path.isEmpty || components.path == "/" else { throw NodStoreError.invalidEnrollmentLink }
    let items = components.queryItems ?? []
    guard Set(items.map(\.name)).count == items.count,
          let server = items.first(where: { $0.name == "server" })?.value,
          let serverComponents = URLComponents(string: server),
          ["http", "https"].contains(serverComponents.scheme?.lowercased() ?? ""),
          serverComponents.host?.isEmpty == false,
          serverComponents.user == nil, serverComponents.password == nil,
          serverComponents.query == nil, serverComponents.fragment == nil,
          let code = items.first(where: { $0.name == "code" })?.value?.uppercased(),
          code.count == 8, code.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else {
      throw NodStoreError.invalidEnrollmentLink
    }
    self.serverURL = server
    self.code = code
    self.deviceName = items.first(where: { $0.name == "name" })?.value?.trimmingCharacters(in: .whitespacesAndNewlines)
  }
}
