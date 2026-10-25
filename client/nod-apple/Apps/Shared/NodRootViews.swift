import NodKit
import SwiftUI

#if os(macOS)
import AppKit
#endif

struct NodRootView: View {
  @EnvironmentObject private var store: NodStore

  var body: some View {
    Group {
      #if os(macOS)
      // Reserve the footer's intrinsic height outside the split view: a root
      // safe-area inset can otherwise cover the detail's final answer option.
      VStack(spacing: 0) {
        inboxContent
          .frame(maxWidth: .infinity, maxHeight: .infinity)
        if store.isRegistered { ConnectionStatusView() }
      }
      #else
      // iOS shows connection state as a navigation-bar indicator: a bottom
      // inset covers the tab bar and the detail's final controls.
      inboxContent
      #endif
    }
    .task {
      #if os(macOS)
        await store.requestNotifications(reportMissingGrant: false)
      #endif
      if store.isRegistered {
        await store.refresh()
        store.prepareAnswerStorage()
        store.connectSync()
      }
    }
    .onOpenURL { store.importEnrollmentLink($0) }
    .onChange(of: store.isRegistered) { _, registered in
      if registered { store.prepareAnswerStorage() }
    }
    .alert("Nod", isPresented: Binding(
      get: { store.alertMessage != nil },
      set: { if !$0 { store.dismissAlertMessage() } }
    )) {
      if store.canReEnrollInvalidSession {
        Button("Re-enroll Device") {
          store.beginInvalidSessionReEnrollment()
        }
      }
      Button("OK", role: .cancel) {}
    } message: {
      Text(store.alertMessage ?? "")
    }
  }

  private var inboxContent: some View {
    Group {
      if store.isRegistered {
        #if os(iOS)
          NodPhoneInbox()
        #else
          NodDesktopInbox()
        #endif
      } else {
        NavigationStack {
          RegistrationView()
        }
      }
    }
  }
}

#if os(iOS)
private enum InboxRoute: Hashable { case all; case channel(String); case request(String) }

struct NodPhoneInbox: View {
  @EnvironmentObject private var store: NodStore
  @State private var showingRegistration = false
  @State private var showingSubscriptions = false
  @State private var showingDevices = false
  @State private var path: [InboxRoute] = []
  @State private var handledNotificationOpenRequestId: UUID?
  @State private var selectedTab = 0

  var body: some View {
    TabView(selection: $selectedTab) {
      NavigationStack(path: $path) {
        ChannelRequestsView(channelId: nil)
          .toolbar {
            ToolbarItem(placement: .topBarLeading) {
              Menu(store.selectedServer?.name ?? "Server") {
                ForEach(store.servers) { server in
                  Button(server.name) { path = []; store.selectServer(server.id) }
                }
              }
            }
            ToolbarItem(placement: .topBarLeading) { ConnectionStatusIndicator() }
          }
          .navigationDestination(for: InboxRoute.self) { route in
            switch route {
            case .all: ChannelRequestsView(channelId: nil)
            case .channel(let channelId): ChannelRequestsView(channelId: channelId)
            case .request(let requestId): RequestDetailContainer(requestId: requestId, explicitRead: true)
            }
          }
      }
      .tabItem { Label("Inbox", systemImage: "tray") }.tag(0)
      NavigationStack {
        List { channelsSection }
          .navigationTitle("Channels")
          .navigationDestination(for: InboxRoute.self) { route in
            switch route {
            case .all: ChannelRequestsView(channelId: nil)
            case .channel(let id): ChannelRequestsView(channelId: id)
            case .request(let id): RequestDetailContainer(requestId: id, explicitRead: true)
            }
          }
      }
      .tabItem { Label("Channels", systemImage: "number") }.tag(1)
      NavigationStack {
        List {
          serversSection
          Section {
            Button("Devices") { showingDevices = true }
            Button("Subscriptions") { showingSubscriptions = true }
            Button("Add Server") { showingRegistration = true }
          }
          appBuildSection
        }
        .navigationTitle("Settings")
      }
      .tabItem { Label("Settings", systemImage: "gearshape") }.tag(2)
    }
    .onAppear {
      applyNotificationOpenRequest(store.notificationOpenRequest)
    }
    .onChange(of: store.notificationOpenRequest) { _, request in
      applyNotificationOpenRequest(request)
    }
    .onChange(of: store.registrationPromptRequestId) { _, requestId in
      if requestId != nil {
        showingRegistration = true
      }
    }
    .sheet(isPresented: $showingRegistration) {
      NavigationStack {
        RegistrationView()
      }
    }
    .sheet(isPresented: $showingSubscriptions) {
      SubscriptionSheet()
    }
    .sheet(isPresented: $showingDevices) {
      DeviceManagementSheet()
    }
  }

  private func applyNotificationOpenRequest(_ request: NodNotificationOpenRequest?) {
    guard
      let request,
      handledNotificationOpenRequestId != request.id,
      let channelId = request.channelId,
      !channelId.isEmpty
    else {
      return
    }
    handledNotificationOpenRequestId = request.id
    selectedTab = 0
    path = []
    if let requestId = request.requestId { path.append(.request(requestId)) }
  }

  private var serversSection: some View {
    Section("Servers") {
      ForEach(store.servers) { server in
        Button {
          path = []
          store.selectServer(server.id)
        } label: {
          HStack {
            Label(server.name, systemImage: "server.rack")
            Spacer()
            ServerStatusIcon(
              isSelected: server.id == store.selectedServer?.id,
              connectionIssue: store.connectionIssue(for: server)
            )
          }
        }
      }
      .onDelete(perform: deleteServers)
    }
  }

  private func deleteServers(at offsets: IndexSet) {
    let selectedServerId = store.selectedServer?.id
    let serverIds = offsets.compactMap { index in
      store.servers.indices.contains(index) ? store.servers[index].id : nil
    }

    store.forgetServers(serverIds)
    if let selectedServerId, serverIds.contains(selectedServerId) {
      path = []
    }
  }

  private var channelsSection: some View {
    Section("Channels") {
      NavigationLink(value: InboxRoute.all) { Label("All requests", systemImage: "tray.full") }
      if store.subscribedChannels.isEmpty {
        ContentUnavailableView("No Subscribed Channels", systemImage: "number")
      } else {
        ForEach(store.subscribedChannels) { channel in
          NavigationLink(value: InboxRoute.channel(channel.id)) {
            ChannelRow(
              channel: channel,
              pendingCount: store.pendingCountsByChannel[channel.id, default: 0]
            )
          }
        }
      }
    }
  }

  private var appBuildSection: some View {
    Section {
      AppBuildLabel()
        .frame(maxWidth: .infinity, alignment: .center)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
    }
  }
}
#else
struct NodDesktopInbox: View {
  @EnvironmentObject private var store: NodStore
  @State private var showingRegistration = false
  @State private var showingSubscriptions = false
  @State private var showingDevices = false

  var body: some View {
    NavigationSplitView {
      NodSidebar(
        showingRegistration: $showingRegistration,
        showingSubscriptions: $showingSubscriptions,
        showingDevices: $showingDevices
      )
      .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 280)
    } content: {
      RequestListView()
        .navigationSplitViewColumnWidth(min: 230, ideal: 320, max: 400)
    } detail: {
      RequestDetailContainer(requestId: store.selectedRequestId)
    }
    .sheet(isPresented: $showingRegistration) {
      NavigationStack {
        RegistrationView()
      }
    }
    .sheet(isPresented: $showingSubscriptions) {
      SubscriptionSheet()
    }
    .sheet(isPresented: $showingDevices) {
      DeviceManagementSheet()
    }
    .onChange(of: store.registrationPromptRequestId) { _, requestId in
      if requestId != nil {
        showingRegistration = true
      }
    }
  }
}

struct NodSidebar: View {
  @EnvironmentObject private var store: NodStore
  @Binding var showingRegistration: Bool
  @Binding var showingSubscriptions: Bool
  @Binding var showingDevices: Bool
  @State private var serverPendingRemoval: NodServerProfile?

  var body: some View {
    List {
      Section("Servers") {
        ForEach(store.servers) { server in
          serverRow(for: server)
        }
      }

      Section("Channels") {
        Button { store.selectChannelExplicitly(nil) } label: { Label("All requests", systemImage: "tray.full") }
          .buttonStyle(.plain)
        if store.subscribedChannels.isEmpty {
          Text("No subscribed channels")
            .foregroundStyle(.secondary)
        } else {
          ForEach(store.subscribedChannels) { channel in
            Button {
              store.selectChannelExplicitly(channel.id)
            } label: {
              ChannelRow(
                channel: channel,
                pendingCount: store.pendingCountsByChannel[channel.id, default: 0]
              )
            }
            .buttonStyle(.plain)
          }
        }
      }

      Section {
        AppBuildLabel()
          .frame(maxWidth: .infinity, alignment: .center)
          .listRowBackground(Color.clear)
          .listRowSeparator(.hidden)
      }
    }
    .confirmationDialog(
      "Forget Server",
      isPresented: Binding(
        get: { serverPendingRemoval != nil },
        set: { if !$0 { serverPendingRemoval = nil } }
      ),
      presenting: serverPendingRemoval
    ) { server in
      Button("Forget Server", role: .destructive) {
        store.forgetServers([server.id])
        serverPendingRemoval = nil
      }
    } message: { server in
      Text("This removes \(server.name) and its saved token from this Mac. It does not revoke the device on the server.")
    }
    .navigationTitle("Nod")
    .toolbar {
      ToolbarItemGroup {
        Button {
          Task { await store.refresh(); store.connectSync() }
        } label: {
          Label("Refresh", systemImage: "arrow.clockwise")
        }
        Button {
          showingDevices = true
        } label: {
          Label("Devices", systemImage: "person.crop.circle")
        }
        Button {
          showingSubscriptions = true
        } label: {
          Label("Subscriptions", systemImage: "slider.horizontal.3")
        }
        Button {
          showingRegistration = true
        } label: {
          Label("Add Server", systemImage: "plus")
        }
      }
    }
  }

  private func serverRow(for server: NodServerProfile) -> some View {
    HStack(spacing: 6) {
      Button {
        store.selectServer(server.id)
      } label: {
        HStack {
          Label(server.name, systemImage: "server.rack")
          Spacer()
          ServerStatusIcon(
            isSelected: server.id == store.selectedServer?.id,
            connectionIssue: store.connectionIssue(for: server)
          )
        }
        .contentShape(Rectangle())
        .frame(maxWidth: .infinity, alignment: .leading)
      }
      .buttonStyle(.plain)

      Menu {
        serverCommands(for: server)
      } label: {
        Label("Server Commands", systemImage: "ellipsis.circle")
      }
      .labelStyle(.iconOnly)
      .menuStyle(.borderlessButton)
      .controlSize(.small)
      .help("Server Commands")
    }
    .contextMenu {
      serverCommands(for: server)
    }
  }

  @ViewBuilder
  private func serverCommands(for server: NodServerProfile) -> some View {
    Button {
      store.selectServer(server.id)
      showingDevices = true
    } label: {
      Label("Manage Devices", systemImage: "person.crop.circle")
    }

    Divider()

    Button(role: .destructive) {
      serverPendingRemoval = server
    } label: {
      Label("Forget Server", systemImage: "trash")
    }
  }
}
#endif

struct ServerStatusIcon: View {
  let isSelected: Bool
  let connectionIssue: String?

  var body: some View {
    if let connectionIssue {
      Image(systemName: "exclamationmark.circle.fill")
        .foregroundStyle(.red)
        .help(connectionIssue)
        .accessibilityLabel("Server connection issue")
        .accessibilityValue(connectionIssue)
    } else if isSelected {
      Image(systemName: "checkmark")
        .foregroundStyle(.tint)
        .accessibilityLabel("Selected server")
    }
  }
}

struct ConnectionStatusView: View {
  @EnvironmentObject private var store: NodStore

  var body: some View {
    HStack(spacing: 8) {
      Image(systemName: store.isSyncConnected ? "checkmark.circle" : "wifi.slash")
      VStack(alignment: .leading, spacing: 2) {
        Text(statusLabel).font(.caption.weight(.medium))
        if !store.isSyncConnected, let date = store.lastSyncedAt {
          (Text("Last updated ") + Text(date, style: .relative)).font(.caption2).foregroundStyle(.secondary)
        }
      }
      Spacer()
    }
    .padding(.horizontal).padding(.vertical, 8)
    .background(.bar)
    .accessibilityElement(children: .contain)
  }

  private var statusLabel: String { connectionStatusLabel(store.syncPhase) }
}

private func connectionStatusLabel(_ phase: String) -> String {
  switch phase {
  case "current": return "Connected"
  case "connecting": return "Connecting…"
  case "reconciling": return "Updating requests…"
  case "revoked": return "Device access revoked"
  default: return "Offline — requests may be out of date"
  }
}

#if os(iOS)
/// Compact connection state for the iOS navigation bar; the menu carries the
/// full status, last update and a refresh that also retries the sync socket.
struct ConnectionStatusIndicator: View {
  @EnvironmentObject private var store: NodStore

  var body: some View {
    Menu {
      Section {
        Text(connectionStatusLabel(store.syncPhase))
        if !store.isSyncConnected, let date = store.lastSyncedAt {
          Text("Last updated \(date.formatted(.relative(presentation: .named)))")
        }
      }
      Button {
        Task { await store.refresh(); store.connectSync() }
      } label: {
        Label("Refresh", systemImage: "arrow.clockwise")
      }
      .disabled(store.isRefreshing)
    } label: {
      icon
    }
    .accessibilityLabel("Connection")
    .accessibilityValue(connectionStatusLabel(store.syncPhase))
  }

  @ViewBuilder private var icon: some View {
    switch store.syncPhase {
    case "current":
      Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
    case "connecting", "reconciling":
      Image(systemName: "arrow.triangle.2.circlepath").foregroundStyle(.secondary)
    case "revoked":
      Image(systemName: "xmark.octagon.fill").foregroundStyle(.red)
    default:
      Image(systemName: "wifi.slash").foregroundStyle(.orange)
    }
  }
}
#endif
