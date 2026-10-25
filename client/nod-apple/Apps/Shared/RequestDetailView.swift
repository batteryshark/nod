import NodKit
import SwiftUI

struct RequestDetail: View {
  @EnvironmentObject private var store: NodStore
  let request: NodRequest
  var serverId: String? = nil
  var explicitRead = false
  @State private var responseDraft: ResponseDraft?
  @State private var showingOptions = false
  @State private var acknowledgedOpenKey: String?
  @State private var destructiveOption: ResponseDraft?

  private struct ResponseDraft: Identifiable {
    let request: NodRequest
    let option: NodRequestOption
    let serverId: String?
    let answerValue: NodAnswerValue?
    var id: String { [serverId ?? "", request.id, option.id].joined(separator: ":") }
    var notesKey: String { NodAnswerComposition.notesKey(serverId: serverId ?? "", requestId: request.id, optionId: option.id, isValueRequest: request.isValueRequest) }
  }

  var body: some View {
    GeometryReader { geometry in
    VStack(spacing: 0) {
    ScrollView {
      VStack(alignment: .leading, spacing: 18) {
        // Badge above the title so a long title gets the full width.
        VStack(alignment: .leading, spacing: 8) {
          StatusBadge(request: request)
          Text(request.title)
            .font(.title2)
            .fontWeight(.semibold)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
          VStack(alignment: .leading, spacing: 2) {
            Text("\(channelName) · \(serverName)")
            Text("\(request.createdAt.formatted(date: .abbreviated, time: .shortened)) · \(resolutionLabel)")
          }
          .font(.caption)
          .foregroundStyle(.secondary)
        }
        Divider()
        if let field = request.answerField, store.needsAnswer(request) {
          RequestValueControl(field: field, value: valueBinding)
            .id(valueScope)
            .disabled(store.answerAttempt(requestId: request.id, serverId: serverId) != nil || store.isSubmitting(request.id))
          if !field.isValid(valueBinding.wrappedValue) {
            Text("Enter a valid \(field.label.lowercased()).").foregroundStyle(.red)
          }
          TextField("Notes (optional)", text: valueNotes, axis: .vertical)
            .lineLimit(2...4)
            .accessibilityIdentifier("answer-value-notes")
            .disabled(store.answerAttempt(requestId: request.id, serverId: serverId) != nil || store.isSubmitting(request.id))
          if store.answerValueDrafts[valueScope] != nil || !valueNotes.wrappedValue.isEmpty {
            Text(store.answerDraftsSaved ? "Draft saved on this device" : "Draft kept while Nod is open").font(.caption).foregroundStyle(.secondary)
          }
          if let error = store.answerStorageError { Text(error).foregroundStyle(.red) }
          if valueNotes.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).utf8.count > 16384 {
            Text("Notes must be 16384 UTF-8 bytes or fewer.").foregroundStyle(.red)
          }
        } else if request.isValueRequest && request.answerField == nil {
          Text("Open in an updated Nod app to answer this request.")
        }
        if store.needsAnswer(request) && !NodStore.isAnswerable(request) {
          Text("Request changed. Review it again before answering.").foregroundStyle(.red)
        }
        if let expiresAt = request.expiresAt, request.status == .pending {
          Label { Text("Expires ") + Text(expiresAt, style: .relative) } icon: { Image(systemName: "clock") }
            .font(.callout)
        }
        if !request.isValueRequest && request.status == .pending && store.answerAttempt(requestId: request.id, serverId: serverId) == nil {
          ForEach(request.options.filter { option in
            let key = [serverId ?? store.selectedServerId ?? "", request.id, option.id].joined(separator: ":")
            return !store.responseDrafts[key, default: ""].isEmpty
          }) { option in
            Button("Resume \(option.label)") { perform(option) }
              .frame(minHeight: 44)
              .disabled(!canAnswer)
          }
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

        if let decision = store.recordedAnswer(in: request) {
          RequestDecisionView(decision: decision, field: request.answerField)
          if request.decisionResolution == .perUser && request.status == .pending {
            Text("Waiting for other recipients").font(.callout)
          }
        }
        #if os(iOS)
        if !request.isValueRequest && !store.acknowledgeOnOpen && request.status == .pending && request.options.isEmpty {
          Button("Acknowledge") { Task { await store.dismissIfInformational(request: request, serverId: serverId) } }
            .disabled(store.isSubmitting(request.id) || !NodStore.isAnswerable(request))
        }
        #endif
      }
      .padding()
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityIdentifier("request-body")
    if let attempt = store.answerAttempt(requestId: request.id, serverId: serverId) {
      VStack(alignment: .leading) {
        Text(attempt.isSubmitting ? "Sending \(attempt.optionLabel)…" : "Could not confirm \(attempt.optionLabel)")
        if let value = attempt.answerValue {
          Text(request.answerField?.choices?.first(where: { $0.id == value.string })?.label ?? value.string)
        }
        Button("Check status") {
          Task { await store.checkAnswerStatus(requestId: request.id, serverId: attempt.serverId) }
        }
        .disabled(attempt.isSubmitting)
      }
      .padding().frame(maxWidth: .infinity, alignment: .leading).background(.bar)
    } else if request.status == .pending && store.recordedAnswer(in: request) == nil && !request.options.isEmpty {
      RequestDecisionTray(options: request.options, isSubmitting: !canAnswer, perform: perform) {
        showingOptions = true
      }
      .frame(maxHeight: geometry.size.height * 0.35)
      .fixedSize(horizontal: false, vertical: true)
    }
    }
    .sheet(isPresented: Binding(get: { showingOptions || responseDraft != nil || destructiveOption != nil }, set: {
      if !$0 { showingOptions = false; responseDraft = nil; destructiveOption = nil }
    })) {
      ZStack {
        RequestAnswerSheet(options: request.options, isSubmitting: !canAnswer, perform: perform, isComposing: responseDraft != nil || destructiveOption != nil)
          .opacity(responseDraft == nil && destructiveOption == nil ? 1 : 0)
          .allowsHitTesting(responseDraft == nil && destructiveOption == nil)
          .accessibilityHidden(responseDraft != nil || destructiveOption != nil)
        if let draft = responseDraft { composer(draft) }
        if let selection = destructiveOption {
          RequestDestructiveAnswerConfirmation(requestTitle: selection.request.title, option: selection.option, valueLabel: selection.answerValue.map { value in selection.request.answerField?.choices?.first(where: { $0.id == value.string })?.label ?? value.string }) {
            destructiveOption = nil
          } confirm: {
            destructiveOption = nil
            performConfirmed(selection)
          }
        }
      }
      #if os(macOS)
      .frame(width: max(280, min(520, geometry.size.width)), height: geometry.size.height * 0.85)
      #else
      .presentationDetents([.large])
      #endif
    }
    }
    #if os(iOS)
    // The body header carries the full title; the bar only gives context.
    .navigationTitle(channelName)
    .navigationBarTitleDisplayMode(.inline)
    #else
    .navigationTitle(request.title)
    #endif
    .task(id: request.id) {
      store.prepareAnswerStorage()
      #if !os(macOS)
      let openKey = (serverId ?? store.selectedServerId ?? "") + ":" + request.id
      if explicitRead && store.acknowledgeOnOpen && acknowledgedOpenKey != openKey {
        acknowledgedOpenKey = openKey
        await store.dismissIfInformational(request: request, serverId: serverId)
      }
      #endif
    }
    .onChange(of: store.selectedServerId) { showingOptions = false; responseDraft = nil; destructiveOption = nil }
  }

  private var serverName: String {
    store.servers.first { $0.id == serverId }?.name ?? store.selectedServer?.name ?? "Nod"
  }

  private var channelName: String {
    store.channels.first { $0.id == request.channelId }?.name ?? request.channelId
  }

  private var resolutionLabel: String {
    request.options.isEmpty && !request.isValueRequest ? "Information" : (request.decisionResolution == .shared ? "Shared decision" : "Your answer")
  }

  private func composer(_ draft: ResponseDraft) -> some View {
      NavigationStack {
        Form {
          Section(draft.request.title) {
            if let value = draft.answerValue {
              Text(draft.request.answerField?.choices?.first(where: { $0.id == value.string })?.label ?? value.string)
            }
            TextField(draft.option.textPlaceholder ?? "Notes", text: Binding(
              get: { store.responseDrafts[draft.notesKey, default: ""] },
              set: { store.responseDrafts[draft.notesKey] = $0 }
            ), axis: .vertical)
              .lineLimit(4...8)
              .disabled(store.isSubmitting(draft.request.id))
              .accessibilityIdentifier("answer-text")
            Text("Required").font(.caption).foregroundStyle(.secondary)
            if !store.responseDrafts[draft.notesKey, default: ""].isEmpty {
              Text(store.answerDraftsSaved ? "Draft saved on this device" : "Draft kept while Nod is open").font(.caption).foregroundStyle(.secondary)
            }
            if let error = store.answerStorageError { Text(error).foregroundStyle(.red) }
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
            Button(store.isSubmitting(draft.request.id) ? "Sending…" : "Send answer") {
              let text = store.responseDrafts[draft.notesKey, default: ""]
              Task {
                _ = await store.answerFromUI(request: draft.request, option: draft.option, text: text, serverId: draft.serverId, answerValue: draft.answerValue)
                if responseDraft?.id == draft.id { responseDraft = nil; showingOptions = false }
              }
            }
            .disabled(!canAnswer || !NodAnswerComposition.canSend(text: store.responseDrafts[draft.notesKey, default: ""], isSubmitting: store.isSubmitting(draft.request.id)) || (request.isValueRequest && store.responseDrafts[draft.notesKey, default: ""].trimmingCharacters(in: .whitespacesAndNewlines).utf8.count > 16384))
            .accessibilityIdentifier("send-answer")
          }
        }
        .interactiveDismissDisabled(store.isSubmitting(draft.request.id))
      }
      .frame(minWidth: 300, minHeight: 240)
  }

  private func perform(_ option: NodRequestOption) {
    let selection = ResponseDraft(request: request, option: option, serverId: serverId ?? store.selectedServerId, answerValue: request.answerField == nil ? nil : valueBinding.wrappedValue)
    // A rejection is not deletion. Only an issuer-marked destructive custom
    // operation adds a scoped confirmation; ordinary approval/rejection stays direct.
    if option.destructive && option.kind == .custom {
      destructiveOption = selection
    } else { performConfirmed(selection) }
  }

  private func performConfirmed(_ selection: ResponseDraft) {
    let request = selection.request
    let option = selection.option
    if option.requiresText {
      responseDraft = selection
    } else {
      showingOptions = false
      let notes = request.answerField == nil ? nil : valueNotes.wrappedValue
      Task { await store.answerFromUI(request: request, option: option, text: notes, serverId: selection.serverId, answerValue: selection.answerValue) }
    }
  }

  private var valueScope: String { [serverId ?? store.selectedServerId ?? "", request.id, request.signing?.requestDigest ?? ""].joined(separator: ":") }
  private var valueBinding: Binding<NodAnswerValue> {
    Binding(get: { store.answerValueDrafts[valueScope] ?? request.answerField?.defaultValue ?? .number("0") },
      set: { store.answerValueDrafts[valueScope] = $0 })
  }
  private var valueNotes: Binding<String> {
    let key = NodAnswerComposition.notesKey(serverId: serverId ?? store.selectedServerId ?? "", requestId: request.id, optionId: "", isValueRequest: true)
    return Binding(get: { store.responseDrafts[key, default: ""] }, set: { store.responseDrafts[key] = $0 })
  }
  private var canAnswer: Bool {
    guard store.needsAnswer(request), NodStore.isAnswerable(request), !store.isSubmitting(request.id), store.answerAttempt(requestId: request.id, serverId: serverId) == nil else { return false }
    guard request.isValueRequest else { return true }
    guard let field = request.answerField else { return false }
    return field.isValid(valueBinding.wrappedValue) && valueNotes.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).utf8.count <= 16384
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

struct RequestDecisionView: View {
  let decision: NodDecision
  var field: NodAnswerField? = nil

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Handled")
        .font(.headline)
      Text(displayLabel)
      if let value = decision.answerValue {
        if let field { Text(field.label).font(.caption).foregroundStyle(.secondary) }
        Text(field?.choices?.first(where: { $0.id == value.string })?.label ?? value.string)
          .textSelection(.enabled)
      }
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
