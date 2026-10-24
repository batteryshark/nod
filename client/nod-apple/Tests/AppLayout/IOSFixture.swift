import SwiftUI
import UIKit

// Isolated test application ONLY. No real NodStore/runtime, credentials,
// network, signing, notification registration or production bundle identifier.
@MainActor final class NodStore: ObservableObject {
  var servers: [NodServerProfile] = []
  var channels: [NodChannel] = []
  var selectedServer: NodServerProfile? { nil }
  var selectedServerId: String? { "fixture" }
  @Published var responseDrafts: [String:String] = [:]
  @Published var answerValueDrafts: [String:NodAnswerValue] = [:]
  @Published var writes = 0
  @Published var lastValue = "none"
  var lastError: String? { nil }
  var answerStorageError: String? { nil }
  var answerDraftsSaved = false
  var acknowledgeOnOpen = false
  func prepareAnswerStorage() {}
  func needsAnswer(_ request: NodRequest) -> Bool { true }
  func recordedAnswer(in request: NodRequest) -> NodDecision? { nil }
  func answerAttempt(requestId: String, serverId: String? = nil) -> NodAnswerAttempt? { nil }
  func isSubmitting(_ id: String) -> Bool { false }
  func dismissIfInformational(request: NodRequest, serverId: String?) async { fatalError("NO ACK IN FIXTURE") }
  func checkAnswerStatus(requestId: String, serverId: String) async -> Bool { fatalError("NO NETWORK IN FIXTURE") }
  func answerFromUI(request: NodRequest, option: NodRequestOption, text: String? = nil, serverId: String?, answerValue: NodAnswerValue? = nil) async -> Bool {
    writes += 1 // Synthetic activation recorder, NEVER a runtime/HTTP write.
    lastValue = answerValue?.string ?? "none"
    return false
  }
}

@main struct NativeAnswerFixture: App {
  @StateObject private var store = NodStore()
  private var request: NodRequest {
    let options = (1...20).map { n in NodRequestOption(id:"option-\(n)", label:"Option \(n): Keep the entire synthetic maintenance record available before choosing this alternative", kind:.custom, requiresText:n == 17, textPlaceholder:"Synthetic required notes", destructive:n == 20) }
    let args = ProcessInfo.processInfo.arguments
    var field: NodAnswerField?
    if let i = args.firstIndex(of:"--field"), args.count > i + 1 {
      switch args[i + 1] {
      case "choice": field = NodAnswerField(kind:.choice, label:"Batch", choices:[.init(id:"small",label:"Small"),.init(id:"large",label:"Large")], defaultValue:.choice("small"))
      case "number": field = NodAnswerField(kind:.number, label:"Count", defaultValue:.number("4"))
      case "date": field = NodAnswerField(kind:.date, label:"Day", defaultValue:.date("2028-02-29"))
      case "date_time": field = NodAnswerField(kind:.dateTime, label:"Time", defaultValue:.dateTime("2026-10-25T09:30:00Z"))
      default: break
      }
    }
    return NodRequest(id:"synthetic",requestId:"synthetic",channelId:"fixture",recipients:[],decisionResolution:.shared,title:"Synthetic native reachability",summary:"",bodyMarkdown:(1...12).map { "## Section \($0)\n\n" + String(repeating:"Complete synthetic reading content. ",count:12) }.joined(separator:"\n\n") + "\n\nReader tail retained.",fields:[],links:[],imageUrl:nil,notification:.init(),dedupeKey:nil,expiresAt:nil,status:.pending,createdAt:Date(timeIntervalSince1970:0),updatedAt:Date(timeIntervalSince1970:0),resolvedAt:nil,decision:nil,decisions:[],callbackUrl:nil,options:options,requestDigest:nil,signing:field == nil ? nil : .init(version:"nod-request-v3",recipientsCommitment:"synthetic",requestDigest:"synthetic-rendered-digest"),answerField:field)
  }
  var body: some Scene {
    WindowGroup {
      NavigationStack { RequestDetail(request:request) }
        .environmentObject(store)
        .environment(\.dynamicTypeSize, ProcessInfo.processInfo.arguments.contains("--large-text") ? .accessibility3 : .large)
        .overlay(alignment:.top) {
          VStack {
            Text("Fixture activations \(store.writes)").accessibilityIdentifier("fixture-activations")
            Text("Fixture value \(store.lastValue)")
          }.font(.caption)
        }
    }
  }
}
