import AppKit
import SwiftUI

@MainActor final class NodStore: ObservableObject {
  var servers: [NodServerProfile] = []
  var channels: [NodChannel] = []
  var macInboxRequests: [NodRequest] = []
  var requests: [NodRequest] { macInboxRequests }
  var selectedRequestId: String?
  var answerAttempts: [String:NodAnswerAttempt] = [:]
  var answerReceipt: NodAnswerReceipt?
  var answerContext: NodAnswerReceipt?
  var answerDraftsSaved = false
  var answerStorageError: String?
  func recordedAnswer(in request: NodRequest) -> NodDecision? { request.decision }
  func answerAttempt(requestId: String, serverId: String? = nil) -> NodAnswerAttempt? { nil }
  func prepareAnswerStorage() {}
  static func isAnswerable(_ request: NodRequest) -> Bool { request.signing?.requestDigest != nil }
  func answerFromUI(request: NodRequest, option: NodRequestOption, text: String? = nil, serverId: String?, answerValue: NodAnswerValue? = nil) async -> Bool {
    fixtureActivations += 1 // Inert recorder only; no runtime/network/signing exists.
    return false
  }
  func checkAnswerStatus(requestId: String, serverId: String) async -> Bool { fatalError("NO NETWORK") }
  func selectRequestExplicitly(_ id: String) { fatalError("NO NAVIGATION IN GEOMETRY PROBE") }
  func macNextPending(excluding id: String?) -> [NodRequest] {
    NodRequestInbox.newestFirst(macInboxRequests.filter { needsAnswer($0) && $0.id != id })
  }
  func selectNextMacPending(excluding id: String?) { fatalError("NO NAVIGATION IN GEOMETRY PROBE") }
  func leaveExplicitRead() { fatalError("NO ACK") }
  var selectedServer: NodServerProfile? { nil }
  var selectedServerId: String? { nil }
  @Published var responseDrafts: [String:String] = [:]
  @Published var fixtureActivations = 0
  var answerValueDrafts: [String:NodAnswerValue] = [:]
  func needsAnswer(_ request: NodRequest) -> Bool { request.status == .pending && request.decision == nil }
  var lastError: String? { nil }
  var isRegistered = true
  var isSyncConnected: Bool { true }
  var lastSyncedAt: Date? { Date(timeIntervalSince1970:0) }
  var isRefreshing: Bool { false }
  var syncPhase: String { "current" }
  func refresh() async { fatalError("NO NETWORK") }
  func connectSync() { fatalError("NO NETWORK") }
  func isSubmitting(_ id: String) -> Bool { false }
  func dismissIfInformational(request: NodRequest, serverId: String?) async { fatalError("NO WRITES") }
  func submit(request: NodRequest, option: NodRequestOption, text: String? = nil, serverId: String?) async -> Bool { fatalError("NO WRITES") }
}
@MainActor enum Registry {
  static var views: [String:NSView] = [:]
}
struct Marker: NSViewRepresentable {
  let id: String
  func makeNSView(context: Context) -> NSView { let v = NSView(); Registry.views[id] = v; return v }
  func updateNSView(_ v: NSView, context: Context) { Registry.views[id] = v }
}
func allViews(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(allViews) }
@MainActor func settle(_ window: NSWindow) {
  for _ in 0..<8 { window.contentView?.layoutSubtreeIfNeeded(); RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.035)) }
}
func rect(_ r: NSRect) -> [String:Double] { ["x":r.minX,"y":r.minY,"width":r.width,"height":r.height] }
private struct FixtureCounter: View {
  @ObservedObject var store: NodStore
  var body: some View { Text("INERT fixture activations: \(store.fixtureActivations)").font(.caption).padding(6) }
}
@MainActor private final class FixtureWindowDelegate: NSObject, NSWindowDelegate {
  func windowWillClose(_ notification: Notification) { NSApplication.shared.terminate(nil) }
}
@MainActor func capture(_ host: NSView, arguments: [String]) throws {
  guard let index = arguments.firstIndex(of: "--png"), index + 1 < arguments.count,
    let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return }
  host.cacheDisplay(in: host.bounds, to: bitmap)
  if let data = bitmap.representation(using: .png, properties: [:]) {
    try data.write(to: URL(fileURLWithPath: arguments[index + 1]))
  }
}

@main struct Probe {
  @MainActor static func main() throws {
    let args = CommandLine.arguments
    // D18: prepared ONLY for the principal install handoff after its 30-second
    // heads-up. Never invoke this mode during agent source checks.
    let visibleHandoff = args.contains("--principal-handoff-visible")
    NSApplication.shared.setActivationPolicy(visibleHandoff ? .regular : .prohibited)
    let count = Int(args[1])!, width = Double(args[2])!, height = Double(args[3])!, paragraphs = Int(args[4])!
    let split = args.contains("--split"), long = args.contains("--long")
    let options = (1...count).map { n in
      NodRequestOption(id:"option-\(n)",label:long ? "Option \(n): Review the entire proposed maintenance schedule and preserve every existing safety constraint before choosing this alternative" : "Option \(n)",kind:.custom,requiresText:visibleHandoff && n == 17,textPlaceholder:"Synthetic required notes",destructive:n == count)
    }
    let request = NodRequest(id:"synthetic",requestId:"synthetic",channelId:"fixture",recipients:[],decisionResolution:.shared,title:"Synthetic reachability fixture",summary:"",bodyMarkdown:(0..<paragraphs).map { "Paragraph \($0). " + String(repeating:"Synthetic content for layout only. ",count:8) }.joined(separator:"\n\n"),fields:[],links:[],imageUrl:nil,notification:.init(),dedupeKey:nil,expiresAt:nil,status:.pending,createdAt:Date(timeIntervalSince1970:0),updatedAt:Date(timeIntervalSince1970:0),resolvedAt:nil,decision:nil,decisions:[],callbackUrl:nil,options:options,requestDigest:nil,signing:.init(version:"nod-request-v1",recipientsCommitment:"synthetic",requestDigest:"synthetic-rendered-digest"))
    let store = NodStore()
    store.macInboxRequests = [request]
    // Many-option content uses the exact bounded-sheet component. This is not
    // proof of the native sheet's presentation or dismissal/focus restoration.
    let detail: AnyView = count > 3 && !visibleHandoff
      ? AnyView(RequestAnswerSheet(options: options, isSubmitting: false, perform: { _ in fatalError("NO WRITES") }).environmentObject(store))
      : AnyView(RequestDetailContainer(requestId: request.id).environmentObject(store))
    let root: AnyView = split ? AnyView(NavigationSplitView { List { Text("Synthetic server"); Text("All requests") } } content: { List { Text("Synthetic request") } } detail: { detail }) : AnyView(detail)
    let composed = ProbeRoot(inboxContent:root).environmentObject(store).frame(minWidth:split ? 860 : 0,minHeight:split ? 560 : 0)
    let controller = NSHostingController(rootView: AnyView(composed))
    let host = controller.view
    let window = NSWindow(contentRect:NSRect(x:-20000,y:-20000,width:width,height:height),styleMask:[.titled,.resizable],backing:.buffered,defer:false)
    window.isReleasedWhenClosed = false
    window.contentViewController = controller
    if visibleHandoff {
      controller.rootView = AnyView(composed.overlay(alignment:.bottomTrailing) { FixtureCounter(store:store) })
      window.title = "Nod INERT fixture — no runtime, network or signing"
      window.minSize = NSSize(width:860,height:560)
      window.setContentSize(NSSize(width:width,height:height))
      window.center()
      let delegate = FixtureWindowDelegate()
      window.delegate = delegate
      window.makeKeyAndOrderFront(nil)
      NSApplication.shared.activate(ignoringOtherApps:true)
      withExtendedLifetime(delegate) { NSApplication.shared.run() }
      return
    }
    window.setContentSize(NSSize(width:width,height:height)); settle(window)
    // SwiftUI installs its toolbar asynchronously; reassert the requested
    // content size after that installation rather than testing a larger host.
    window.setContentSize(NSSize(width:width,height:height)); settle(window)
    if args.contains("--resize") {
      window.setContentSize(NSSize(width:1200,height:800)); settle(window)
      window.setContentSize(NSSize(width:width,height:height)); settle(window)
    }
    if Registry.views["option-1"] == nil, let access = Registry.views["answer-access"] {
      let r = access.convert(access.bounds, to: host)
      let bar = Registry.views["status-bar"]!
      let result: [String:Any] = ["mode":"compact-access", "accessReachable":host.bounds.contains(r) && !r.intersects(bar.convert(bar.bounds, to: host)) && r.height >= 44, "visibleWindow":window.isVisible, "access":rect(r)]
      try capture(host, arguments: args)
      print(String(decoding:try JSONSerialization.data(withJSONObject:result,options:[.prettyPrinted,.sortedKeys]),as:UTF8.self))
      window.close()
      return
    }
    let scroll = allViews(host).compactMap { $0 as? NSScrollView }.first { Registry.views["option-1"]!.isDescendant(of:$0) }
    let doc = scroll?.documentView ?? host
    let bar = Registry.views["status-bar"]!
    let barRect = bar.convert(bar.bounds,to:host)
    let frames = (1...count).map { n -> NSRect in let v = Registry.views["option-\(n)"]!; return v.convert(v.bounds,to:doc) }
    let ids = (1...count).map { "option-\($0)" }
    let orderPreserved = zip(frames,frames.dropFirst()).allSatisfy { $0.minY < $1.minY }
    let reachable = (1...count).allSatisfy { n in
      let view = Registry.views["option-\(n)"]!, r = frames[n-1]
      if let scroll {
        let target = NSRect(x:0,y:max(0,r.minY),width:scroll.contentView.bounds.width,height:scroll.contentView.bounds.height)
        scroll.contentView.scroll(to:scroll.contentView.constrainBoundsRect(target).origin)
        scroll.reflectScrolledClipView(scroll.contentView)
      }
      let h = view.convert(view.bounds,to:host)
      return (scroll?.contentView.bounds ?? host.bounds).insetBy(dx:-0.5,dy:-0.5).contains(r) && !h.intersects(barRect)
    }
    let result: [String:Any] = ["everyOptionReachableWithoutStatusOcclusion":reachable,"orderPreserved":orderPreserved,"ids":ids,"markedOptionCount":Registry.views.keys.filter { $0.hasPrefix("option-") }.count,"visibleWindow":window.isVisible,"host":rect(host.bounds),"barInHost":rect(barRect),"clipInHost":rect(scroll?.contentView.convert(scroll!.contentView.bounds,to:host) ?? host.bounds),"options":frames.map(rect)]
    try capture(host, arguments: args)
    print(String(decoding:try JSONSerialization.data(withJSONObject:result,options:[.prettyPrinted,.sortedKeys]),as:UTF8.self))
    window.close()
  }
}
