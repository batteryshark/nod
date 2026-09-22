import NodKit
import SwiftUI

struct ChannelRequestsView: View {
  @EnvironmentObject private var store: NodStore
  let channelId: String?
  @State private var showingHistory = false
  @State private var searchText = ""
  @State private var pendingOnly = false
  @State private var pendingRequestsExpanded = true
  @State private var handledRequestsExpanded = false
  @State private var initializedSectionExpansion = false

  var body: some View {
    Group {
      if pendingRequests.isEmpty && handledRequests.isEmpty {
        ContentUnavailableView(searchText.isEmpty ? "No Requests" : "No Matching Requests", systemImage: "bell.slash")
      } else {
        List {
          requestSections { request in
            NavigationLink {
              RequestDetailContainer(requestId: request.id)
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
      Toggle("Pending only", isOn: $pendingOnly)
      Button("Search History") { showingHistory = true }
    }
    .sheet(isPresented: $showingHistory) { RequestHistorySheet(channelId: store.selectedChannelId) }
    .navigationTitle(channelName)
    .task {
      if store.selectedChannelId != channelId {
        store.selectedChannelId = channelId
      }
    }
    .onAppear {
      initializeSectionExpansionIfNeeded()
    }
    .onChange(of: channelRequests.isEmpty) {
      initializeSectionExpansionIfNeeded()
    }
    .onChange(of: pendingRequests.isEmpty) { _, hasNoPendingRequests in
      initializeSectionExpansionIfNeeded()
      if hasNoPendingRequests {
        handledRequestsExpanded = true
      }
    }
  }

  private var channelRequests: [NodRequest] {
    NodRequestInbox.newestFirst(store.requests.filter { channelId == nil || $0.channelId == channelId })
  }

  private var pendingRequests: [NodRequest] {
    channelRequests.filter { $0.status == .pending && matchesSearch($0, query: searchText) }
  }

  private var handledRequests: [NodRequest] {
    pendingOnly ? [] : channelRequests.filter { $0.status != .pending && matchesSearch($0, query: searchText) }
  }

  private var channelName: String {
    store.channels.first(where: { $0.id == channelId })?.name ?? "All requests"
  }

  @ViewBuilder
  private func requestSections<RowContent: View>(
    @ViewBuilder rowContent: @escaping (NodRequest) -> RowContent
  ) -> some View {
    if !pendingRequests.isEmpty {
      Section {
        if pendingRequestsExpanded {
          ForEach(pendingRequests) { request in
            rowContent(request)
          }
        }
      } header: {
        RequestSectionHeader(
          title: "Pending",
          count: pendingRequests.count,
          isExpanded: $pendingRequestsExpanded
        )
      }
    }
    if !handledRequests.isEmpty {
      Section {
        if handledRequestsExpanded {
          ForEach(handledRequests) { request in
            rowContent(request)
          }
        }
      } header: {
        RequestSectionHeader(
          title: "Handled",
          count: handledRequests.count,
          isExpanded: $handledRequestsExpanded
        )
      }
    }
  }

  private func initializeSectionExpansionIfNeeded() {
    guard !initializedSectionExpansion, !channelRequests.isEmpty else {
      return
    }
    handledRequestsExpanded = pendingRequests.isEmpty
    initializedSectionExpansion = true
  }
}

struct RequestListView: View {
  @EnvironmentObject private var store: NodStore
  @State private var showingHistory = false
  @State private var searchText = ""
  @State private var pendingOnly = false
  @State private var pendingRequestsExpanded = true
  @State private var handledRequestsExpanded = false
  @State private var initializedSectionExpansion = false

  var body: some View {
    Group {
      if pendingRequests.isEmpty && handledRequests.isEmpty {
        ContentUnavailableView(searchText.isEmpty ? "No Requests" : "No Matching Requests", systemImage: "bell.slash")
      } else {
        List(selection: $store.selectedRequestId) {
          requestSections { request in
            RequestRow(request: request)
              .tag(Optional(request.id))
          }
        }
      }
    }
    .searchable(text: $searchText, prompt: "Search requests")
    .toolbar {
      Toggle("Pending only", isOn: $pendingOnly)
      Button("Search History") { showingHistory = true }
    }
    .sheet(isPresented: $showingHistory) { RequestHistorySheet(channelId: store.selectedChannelId) }
    .navigationTitle(selectedChannelName)
    .onAppear {
      initializeSectionExpansionIfNeeded()
    }
    .onChange(of: store.requests.isEmpty) {
      initializeSectionExpansionIfNeeded()
    }
    .onChange(of: pendingRequests.isEmpty) { _, hasNoPendingRequests in
      initializeSectionExpansionIfNeeded()
      if hasNoPendingRequests {
        handledRequestsExpanded = true
      }
    }
    .onChange(of: store.selectedChannelId) {
      pendingRequestsExpanded = true
      handledRequestsExpanded = false
      initializedSectionExpansion = false
      initializeSectionExpansionIfNeeded()
    }
    .onChange(of: store.selectedRequestId) { oldValue, newValue in
      dismissSelectedIfInformational(newValue)
    }
  }

  private func dismissSelectedIfInformational(_ requestId: String?) {
    guard store.acknowledgeOnOpen, let requestId else {
      return
    }
    guard let request = store.requests.first(where: { $0.id == requestId }),
      request.status == .pending,
      request.options.isEmpty
    else {
      return
    }
    Task { await store.dismissIfInformational(request: request) }
  }

  private var selectedChannelName: String {
    store.channels.first(where: { $0.id == store.selectedChannelId })?.name ?? "All requests"
  }

  private var pendingRequests: [NodRequest] {
    NodRequestInbox.newestFirst(store.requests.filter { $0.status == .pending && matchesSearch($0, query: searchText) })
  }

  private var handledRequests: [NodRequest] {
    pendingOnly ? [] : NodRequestInbox.newestFirst(store.requests.filter { $0.status != .pending && matchesSearch($0, query: searchText) })
  }

  @ViewBuilder
  private func requestSections<RowContent: View>(
    @ViewBuilder rowContent: @escaping (NodRequest) -> RowContent
  ) -> some View {
    if !pendingRequests.isEmpty {
      Section {
        if pendingRequestsExpanded {
          ForEach(pendingRequests) { request in
            rowContent(request)
          }
        }
      } header: {
        RequestSectionHeader(
          title: "Pending",
          count: pendingRequests.count,
          isExpanded: $pendingRequestsExpanded
        )
      }
    }
    if !handledRequests.isEmpty {
      Section {
        if handledRequestsExpanded {
          ForEach(handledRequests) { request in
            rowContent(request)
          }
        }
      } header: {
        RequestSectionHeader(
          title: "Handled",
          count: handledRequests.count,
          isExpanded: $handledRequestsExpanded
        )
      }
    }
  }

  private func initializeSectionExpansionIfNeeded() {
    guard !initializedSectionExpansion, !store.requests.isEmpty else {
      return
    }
    handledRequestsExpanded = pendingRequests.isEmpty
    initializedSectionExpansion = true
  }
}

struct RequestDetailContainer: View {
  @EnvironmentObject private var store: NodStore
  let requestId: String?

  var body: some View {
    if let request = request {
      RequestDetail(request: request)
    } else {
      ContentUnavailableView("Select a Request", systemImage: "rectangle.stack.badge.person.crop")
    }
  }

  private var request: NodRequest? {
    guard let requestId else {
      return nil
    }
    return store.requests.first(where: { $0.id == requestId })
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
  let request: NodRequest

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        Text(request.title)
          .font(.headline)
          .lineLimit(2)
          .frame(maxWidth: .infinity, alignment: .leading)
        StatusBadge(request: request)
          .fixedSize()
          .layoutPriority(1)
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

  var body: some View {
    NavigationStack {
      List {
        if let error {
          Text(error).foregroundStyle(.red)
          Button("Retry") { Task { await load(reset: requests.isEmpty) } }
        }
        ForEach(requests) { request in
          NavigationLink { RequestDetail(request: store.requests.first { $0.id == request.id } ?? request, serverId: serverId) } label: { RequestRow(request: request) }
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
        ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
        ToolbarItem(placement: .confirmationAction) { Button("Search") { Task { await load(reset: true) } }.disabled(isLoading) }
      }
      .task { serverId = store.selectedServerId; await load(reset: true) }
      .onChange(of: store.selectedServerId) { dismiss() }
    }
    .frame(minWidth: 320, minHeight: 400)
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
