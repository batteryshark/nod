import NodKit
import SwiftUI

struct RequestDetail: View {
  @EnvironmentObject private var store: NodStore
  let request: NodRequest
  var serverId: String? = nil
  @State private var responseDraft: ResponseDraft?

  private struct ResponseDraft: Identifiable {
    let request: NodRequest
    let option: NodRequestOption
    let serverId: String?
    var id: String { [serverId ?? "", request.id, option.id].joined(separator: ":") }
  }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 18) {
        HStack(alignment: .firstTextBaseline) {
          VStack(alignment: .leading, spacing: 4) {
            Text(request.title)
              .font(.title2)
              .fontWeight(.semibold)
            Text(request.createdAt, format: .dateTime)
              .font(.caption)
              .foregroundStyle(.secondary)
          }
          Spacer()
          StatusBadge(request: request)
        }

        HStack {
          Text(store.servers.first { $0.id == serverId }?.name ?? store.selectedServer?.name ?? "Nod")
          Text("•")
          Text(store.channels.first { $0.id == request.channelId }?.name ?? request.channelId)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        if let expiresAt = request.expiresAt, request.status == .pending {
          Label { Text("Expires ") + Text(expiresAt, style: .relative) } icon: { Image(systemName: "clock") }
            .font(.callout)
        }
        if request.bodyMarkdown.isEmpty, !request.summary.isEmpty { Text(request.summary).textSelection(.enabled) }
        if let imageURL = topImageURL {
          RequestImageView(url: imageURL)
        }

        if !request.bodyMarkdown.isEmpty {
          MarkdownText(markdown: request.bodyMarkdown, ignoredImageURLs: markdownIgnoredImageURLs)
            .textSelection(.enabled)
        }

        if !request.fields.isEmpty {
          Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
            ForEach(request.fields, id: \.self) { field in
              GridRow {
                Text(field.label)
                  .foregroundStyle(.secondary)
                Text(field.value)
                  .textSelection(.enabled)
              }
            }
          }
        }

        if !request.links.isEmpty {
          VStack(alignment: .leading, spacing: 8) {
            ForEach(request.links, id: \.self) { link in
              if let resolvedLink = resolvedLink(link) {
                RequestLinkView(label: resolvedLink.label, url: resolvedLink.url)
              }
            }
          }
        }

        if let decision = request.decision {
          RequestDecisionView(decision: decision)
        } else if request.status == .pending && !request.options.isEmpty {
          RequestOptionArea(options: request.options, isSubmitting: store.isSubmitting(request.id)) { option in
            perform(option)
          }
        } else if request.status == .pending && request.options.isEmpty {
          Button("Acknowledge") { Task { await store.dismissIfInformational(request: request, serverId: serverId) } }
            .disabled(store.isSubmitting(request.id))
        }
      }
      .padding()
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .navigationTitle(request.title)
    .task(id: request.id) {
      if store.acknowledgeOnOpen { await store.dismissIfInformational(request: request, serverId: serverId) }
    }
    .sheet(item: $responseDraft) { draft in
      NavigationStack {
        Form {
          Section(draft.request.title) {
            TextField(draft.option.textPlaceholder ?? "Notes", text: Binding(
              get: { store.responseDrafts[draft.id, default: ""] },
              set: { store.responseDrafts[draft.id] = $0 }
            ), axis: .vertical)
              .lineLimit(4...8)
              .disabled(store.isSubmitting(draft.request.id))
            Text("Notes are optional.").font(.caption).foregroundStyle(.secondary)
          }
          if let error = store.lastError { Text(error).foregroundStyle(.red) }
        }
        .navigationTitle(draft.option.label)
        .toolbar {
          ToolbarItem(placement: .cancellationAction) {
            Button("Cancel") { responseDraft = nil }
              .disabled(store.isSubmitting(draft.request.id))
          }
          ToolbarItem(placement: .confirmationAction) {
            Button(store.isSubmitting(draft.request.id) ? "Sending…" : "Submit") {
              let text = store.responseDrafts[draft.id, default: ""]
              Task {
                if await store.submit(request: draft.request, option: draft.option, text: text, serverId: draft.serverId) {
                  store.responseDrafts.removeValue(forKey: draft.id)
                  if responseDraft?.id == draft.id { responseDraft = nil }
                }
              }
            }
            .disabled(store.isSubmitting(draft.request.id))
          }
        }
        .interactiveDismissDisabled(store.isSubmitting(draft.request.id))
      }
      .frame(minWidth: 300, minHeight: 240)
    }
  }

  private func perform(_ option: NodRequestOption) {
    let serverId = serverId ?? store.selectedServerId
    if option.requiresText {
      responseDraft = ResponseDraft(request: request, option: option, serverId: serverId)
    } else {
      Task { await store.submit(request: request, option: option, serverId: serverId) }
    }
  }

  private var topImageURL: URL? {
    resolvedImageURL(request.imageUrl)
  }

  private var markdownIgnoredImageURLs: Set<String> {
    guard let topImageURL else {
      return []
    }
    return [topImageURL.absoluteString]
  }
}

private struct RequestDecisionView: View {
  let decision: NodDecision

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Handled")
        .font(.headline)
      Text(displayLabel)
      Text(decision.resolvedAt, format: .dateTime)
        .font(.caption).foregroundStyle(.secondary)
      if let actor = decision.actorUserId {
        Text("Responded by \(actor)")
          .font(.caption).textSelection(.enabled)
      }
      if let device = decision.actorDeviceId {
        Text("Device: \(device)")
          .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
      }
      if decision.signature?.verified == true {
        Label("Signature verified by server", systemImage: "checkmark.shield")
          .font(.caption)
      }
      if let text = decision.text, !text.isEmpty {
        Text(text)
          .padding(10)
          .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
      }
    }
  }

  private var displayLabel: String {
    let label = decision.optionLabel.trimmingCharacters(in: .whitespacesAndNewlines)
    if !label.isEmpty { return label }

    switch decision.optionKind {
    case .approve, .approveWithText:
      return "Approved"
    case .reject, .rejectWithText:
      return "Rejected"
    case .dismiss:
      return "Dismissed"
    case .open:
      return "Opened"
    case .custom:
      return "Resolved"
    }
  }
}

private struct RequestOptionArea: View {
  let options: [NodRequestOption]
  let isSubmitting: Bool
  let perform: (NodRequestOption) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      ForEach(options) { option in
        Button(role: option.destructive ? .destructive : nil) { perform(option) } label: {
          Text(option.label)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity, minHeight: 32)
        }
        .buttonStyle(.borderedProminent)
        .disabled(isSubmitting)
      }
      if isSubmitting { ProgressView("Sending response…") }
    }
  }
}
