import NodKit
import SwiftUI

struct ChannelRequestsView: View {
  @EnvironmentObject private var store: NodStore
  let channelId: String?
  @State private var showingHistory = false
  @State private var searchText = ""
  @State private var pendingOnly = true

  var body: some View {
    Group {
      if pendingRequests.isEmpty && handledRequests.isEmpty {
        ContentUnavailableView(searchText.isEmpty ? "No Requests" : "No Matching Requests", systemImage: "bell.slash")
      } else {
        List {
          requestSections { request in
            NavigationLink {
              RequestDetailContainer(requestId: request.id, explicitRead: true)
            } label: {
              RequestRow(request: request)
            }
          }
        }
        .refreshable {
          await store.refresh()
        }
      }
    }
    .searchable(text: $searchText, prompt: "Search requests")
    .toolbar {
      Button("Search history…") { store.answerReceipt = nil; showingHistory = true }
    }
    .safeAreaInset(edge: .top) {
      Picker("Requests", selection: $pendingOnly) {
        Text("Pending").tag(true)
        Text("Handled").tag(false)
      }.pickerStyle(.segmented).padding()
    }
    .sheet(isPresented: $showingHistory) { RequestHistorySheet(channelId: channelId) }
    .navigationTitle(channelName)
    .task {
      if store.selectedChannelId != channelId {
        store.selectedChannelId = channelId
      }
    }
  }

  private var channelRequests: [NodRequest] {
    NodRequestInbox.newestFirst(store.requests.filter { channelId == nil || $0.channelId == channelId })
  }

  private var pendingRequests: [NodRequest] {
    pendingOnly ? channelRequests.filter { store.needsAnswer($0) && matchesSearch($0, query: searchText) } : []
  }

  private var handledRequests: [NodRequest] {
    pendingOnly ? [] : channelRequests.filter { !store.needsAnswer($0) && matchesSearch($0, query: searchText) }
  }

  private var channelName: String {
    store.channels.first(where: { $0.id == channelId })?.name ?? "Inbox"
  }

  @ViewBuilder
  private func requestSections<RowContent: View>(
    @ViewBuilder rowContent: @escaping (NodRequest) -> RowContent
  ) -> some View {
    if !pendingRequests.isEmpty {
      Section("Pending · \(pendingRequests.count)") { ForEach(pendingRequests) { rowContent($0) } }
    }
    if !handledRequests.isEmpty {
      Section("Handled · \(handledRequests.count)") { ForEach(handledRequests) { rowContent($0) } }
    }
  }

}

struct RequestListView: View {
  @EnvironmentObject private var store: NodStore
  @State private var showingHistory = false
  #if os(macOS)
  private var searchText: String { store.macSearchText }
  private var pendingOnly: Bool { store.macPendingOnly }
  private var searchBinding: Binding<String> {
    Binding(get: { searchText }, set: {
      let handled = store.macHandledOnly
      store.setMacInboxFilter(pendingOnly: pendingOnly, searchText: $0)
      if handled { store.setMacInboxMode(handled: true) }
    })
  }
  private var pendingBinding: Binding<Bool> {
    Binding(get: { pendingOnly }, set: { store.setMacInboxFilter(pendingOnly: $0, searchText: searchText) })
  }
  #else
  @State private var searchText = ""
  @State private var pendingOnly = false
  private var searchBinding: Binding<String> { $searchText }
  private var pendingBinding: Binding<Bool> { $pendingOnly }
  #endif

  var body: some View {
    Group {
      if pendingRequests.isEmpty && handledRequests.isEmpty {
        ContentUnavailableView(searchText.isEmpty ? "No Requests" : "No Matching Requests", systemImage: "bell.slash")
      } else {
        List(selection: Binding(get: { store.selectedRequestId }, set: { requestId in
          #if os(macOS)
          store.selectRequestExplicitly(requestId)
          #else
          store.selectedRequestId = requestId
          #endif
        })) {
          requestSections { request in
            RequestRow(request: request)
              .tag(Optional(request.id))
              #if os(macOS)
              .simultaneousGesture(TapGesture().onEnded {
                store.selectRequestExplicitly(request.id)
              })
              #endif
          }
        }
      }
    }
    .searchable(text: searchBinding, prompt: "Search requests")
    .toolbar {
      #if os(macOS)
      Picker("Requests", selection: Binding(get: { store.macHandledOnly }, set: { store.setMacInboxMode(handled: $0) })) {
        Text("Pending").tag(false)
        Text("Handled").tag(true)
      }.pickerStyle(.segmented)
      #else
      Toggle("Pending only", isOn: pendingBinding)
      #endif
      Button("Search History") {
        store.answerReceipt = nil
        #if os(macOS)
        store.leaveExplicitRead()
        #endif
        showingHistory = true
      }
    }
    .sheet(isPresented: $showingHistory) { RequestHistorySheet(channelId: store.selectedChannelId) }
    .navigationTitle(selectedChannelName)
    .onAppear {
      #if os(macOS)
      store.initializeMacInboxUI()
      #endif
    }
  }

  private var selectedChannelName: String {
    store.channels.first(where: { $0.id == store.selectedChannelId })?.name ?? "All requests"
  }

  private var pendingRequests: [NodRequest] {
    NodRequestInbox.newestFirst(inboxRequests.filter { store.needsAnswer($0) && matchesSearch($0, query: searchText) })
  }

  private var handledRequests: [NodRequest] {
    pendingOnly ? [] : NodRequestInbox.newestFirst(inboxRequests.filter { !store.needsAnswer($0) && matchesSearch($0, query: searchText) })
  }

  private var inboxRequests: [NodRequest] {
    #if os(macOS)
    store.macInboxRequests
    #else
    store.requests
    #endif
  }

  @ViewBuilder
  private func requestSections<RowContent: View>(
    @ViewBuilder rowContent: @escaping (NodRequest) -> RowContent
  ) -> some View {
    if !pendingRequests.isEmpty {
      Section("Pending · \(pendingRequests.count)") { ForEach(pendingRequests) { rowContent($0) } }
    }
    if !handledRequests.isEmpty {
      Section("Handled · \(handledRequests.count)") { ForEach(handledRequests) { rowContent($0) } }
    }
  }

}

struct RequestDetailContainer: View {
  @EnvironmentObject private var store: NodStore
  let requestId: String?
  var explicitRead = false
  @State private var nextRequestId: String?

  var body: some View {
    if let receipt = store.answerReceipt, receipt.serverId == store.selectedServerId,
      requestId == nil || receipt.request.id == (nextRequestId ?? requestId) {
      ScrollView {
        VStack(alignment: .leading, spacing: 16) {
          Text(store.selectedServer?.name ?? "Nod").font(.caption).foregroundStyle(.secondary)
          Text(receipt.request.title).font(.title2.bold())
          Text(receipt.request.decisionResolution == .shared ? "Shared decision" : "Your answer")
          if let decision = receipt.decision { RequestDecisionView(decision: decision, field: receipt.request.answerField) }
          else { StatusBadge(request: receipt.request) }
          if receipt.request.decisionResolution == .perUser && receipt.request.status == .pending {
            Text("Waiting for other recipients")
          }
          nextButton
        }.padding().frame(maxWidth: .infinity, alignment: .leading)
      }
    } else if requestId == nil,
      let attempt = store.answerAttempts.values.first(where: { $0.serverId == store.selectedServerId }) {
      if let context = store.answerContext, context.id == attempt.key {
        RequestDetail(request: context.request, serverId: context.serverId)
      } else {
        VStack(spacing: 16) {
          Text(attempt.requestTitle ?? attempt.requestId).font(.headline)
          Text("Could not confirm \(attempt.optionLabel)")
          if let value = attempt.answerValue { Text(value.string) }
          Button("Check status") { Task { await store.checkAnswerStatus(requestId: attempt.requestId, serverId: attempt.serverId) } }
            .disabled(attempt.isSubmitting)
        }.padding()
      }
    } else if let request = request {
      RequestDetail(request: request, explicitRead: explicitRead || nextRequestId != nil)
        .toolbar {
          if store.answerAttempt(requestId: request.id) == nil { nextButton }
        }
    } else {
      ContentUnavailableView("Select a Request", systemImage: "rectangle.stack.badge.person.crop")
    }
  }

  private var request: NodRequest? {
    guard let requestId = nextRequestId ?? requestId else {
      return nil
    }
    #if os(macOS)
    return store.macInboxRequests.first(where: { $0.id == requestId })
    #else
    return store.requests.first(where: { $0.id == requestId })
    #endif
  }

  private var nextPending: [NodRequest] {
    #if os(macOS)
    store.macNextPending(excluding: nextRequestId ?? requestId)
    #else
    NodRequestInbox.newestFirst(store.requests.filter {
      $0.status == .pending && $0.id != (nextRequestId ?? requestId) && store.recordedAnswer(in: $0) == nil
    })
    #endif
  }

  private var nextButton: some View {
    Button("Next pending · \(nextPending.count)") {
      #if os(macOS)
      store.selectNextMacPending(excluding: nextRequestId ?? requestId)
      #else
      store.answerReceipt = nil
      nextRequestId = nextPending.first?.id
      #endif
    }
    .disabled(nextPending.isEmpty)
    .keyboardShortcut("j", modifiers: [.command, .shift])
  }
}

struct ChannelLabel: View {
  let channel: NodChannel

  var body: some View {
    HStack(spacing: 6) {
      Text(channel.emoji.isEmpty ? "🔔" : channel.emoji)
      Text(channel.name)
    }
  }
}

struct ChannelRow: View {
  let channel: NodChannel
  let pendingCount: Int

  var body: some View {
    HStack {
      ChannelLabel(channel: channel)
      Spacer()
      if pendingCount > 0 {
        Text(pendingCount, format: .number)
          .font(.caption)
          .fontWeight(.semibold)
          .monospacedDigit()
          .padding(.horizontal, 7)
          .padding(.vertical, 3)
          .background(Color.accentColor, in: Capsule())
          .foregroundStyle(.white)
          .accessibilityLabel("\(pendingCount) pending")
      }
    }
  }
}

struct RequestRow: View {
  @EnvironmentObject private var store: NodStore
  let request: NodRequest

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        Text(request.title)
          .font(.headline)
          .lineLimit(2)
          .frame(maxWidth: .infinity, alignment: .leading)
        if request.decisionResolution == .perUser && store.recordedAnswer(in: request) != nil {
          Text("Your answer recorded").font(.caption).foregroundStyle(.secondary)
        } else {
          StatusBadge(request: request)
            .fixedSize()
            .layoutPriority(1)
        }
      }
      Text(request.summary)
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .lineLimit(2)
      Text(request.createdAt, style: .relative)
        .font(.caption)
        .foregroundStyle(.tertiary)
    }
    .padding(.vertical, 4)
    .frame(maxWidth: .infinity, alignment: .leading)
    .contentShape(Rectangle())
  }
}

struct RequestSectionHeader: View {
  let title: String
  let count: Int
  @Binding var isExpanded: Bool

  var body: some View {
    Button {
      withAnimation(.default) {
        isExpanded.toggle()
      }
    } label: {
      HStack(spacing: 6) {
        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
          .font(.caption.weight(.semibold))
          .frame(width: 12)
        Text(title)
        Text(count, format: .number)
          .monospacedDigit()
          .foregroundStyle(.secondary)
        Spacer()
      }
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel("\(title), \(count) notifications")
    .accessibilityValue(isExpanded ? "Expanded" : "Collapsed")
  }
}

private func matchesSearch(_ request: NodRequest, query: String) -> Bool {
  let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
  return query.isEmpty || [request.title, request.summary, request.bodyMarkdown, request.id].contains { $0.localizedCaseInsensitiveContains(query) }
}

struct RequestHistorySheet: View {
  @Environment(\.dismiss) private var dismiss
  @EnvironmentObject private var store: NodStore
  let channelId: String?
  @State private var serverId: String?
  @State private var searchText = ""
  @State private var requests: [NodRequest] = []
  @State private var nextCursor: String?
  @State private var isLoading = false
  @State private var error: String?
  #if os(macOS)
  @State private var historyPath: [String] = []
  private var pathBinding: Binding<[String]> {
    Binding(get: { historyPath }, set: { path in
      if path != historyPath {
        store.leaveExplicitRead()
        if let id = path.last, let request = requests.first(where: { $0.id == id }) {
          store.beginExplicitRead(request: request, serverId: serverId)
          store.selectedRequestId = id
        }
      }
      historyPath = path
    })
  }
  #endif

  var body: some View {
    #if os(macOS)
    NavigationStack(path: pathBinding) {
      historyList
        .navigationDestination(for: String.self) { id in
          if let request = store.requests.first(where: { $0.id == id }) ?? requests.first(where: { $0.id == id }) {
            RequestDetail(request: request, serverId: serverId)
          }
        }
    }
    .frame(minWidth: 320, minHeight: 400)
    #else
    NavigationStack {
      historyList
    }
    .frame(minWidth: 320, minHeight: 400)
    #endif
  }

  private var historyList: some View {
      List {
        if let error {
          Text(error).foregroundStyle(.red)
          Button("Retry") { Task { await load(reset: requests.isEmpty) } }
        }
        ForEach(requests) { request in
          #if os(macOS)
          NavigationLink(value: request.id) { RequestRow(request: request) }
          #else
          NavigationLink { RequestDetail(request: store.requests.first { $0.id == request.id } ?? request, serverId: serverId, explicitRead: true) } label: { RequestRow(request: request) }
          #endif
        }
        if isLoading { ProgressView("Loading history…") }
        else if nextCursor != nil { Button("Load Older Requests") { Task { await load(reset: false) } } }
        else if requests.isEmpty { Text("No matching requests.").foregroundStyle(.secondary) }
        else { Text("End of matching history").font(.caption).foregroundStyle(.secondary) }
      }
      .navigationTitle("Request History")
      .searchable(text: $searchText, prompt: "Search server history")
      .onSubmit(of: .search) { Task { await load(reset: true) } }
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Done") {
            #if os(macOS)
            store.leaveExplicitRead()
            #endif
            dismiss()
          }
        }
        ToolbarItem(placement: .confirmationAction) { Button("Search") { Task { await load(reset: true) } }.disabled(isLoading) }
      }
      .task { serverId = store.selectedServerId; await load(reset: true) }
      .onChange(of: store.selectedServerId) { dismiss() }
  }

  private func load(reset: Bool) async {
    guard !isLoading, store.selectedServerId == serverId else { return }
    isLoading = true
    error = nil
    defer { isLoading = false }
    do {
      let page = try await store.queryHistory(serverId: serverId, channelId: channelId, search: searchText, before: reset ? nil : nextCursor)
      guard store.selectedServerId == serverId else { return }
      if reset { requests = [] }
      let loadedIds = Set(requests.map(\.id))
      requests.append(contentsOf: page.requests.filter { !loadedIds.contains($0.id) })
      nextCursor = page.nextCursor
    } catch { self.error = error.localizedDescription }
  }
}
