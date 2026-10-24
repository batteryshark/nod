import NodKit
import SwiftUI

struct RequestValueControl: View {
  let field: NodAnswerField
  @Binding var value: NodAnswerValue
  @State private var editingISODate = false

  var body: some View {
    Group {
      switch field.kind {
      case .choice:
        Picker(field.label, selection: Binding(get: { value.string }, set: { value = .choice($0) })) {
          ForEach(field.choices ?? []) { choice in Text(choice.label).tag(choice.id) }
        }.pickerStyle(.menu)
      case .number:
        LabeledContent(field.label) {
          TextField(field.label, text: Binding(get: { value.string }, set: { value = .number($0) }))
            #if os(iOS)
            .keyboardType(.numbersAndPunctuation)
            #endif
            .textFieldStyle(.roundedBorder)
            .frame(minHeight: 44)
        }
      case .date, .dateTime:
        if editingISODate || NodAnswerField.requiresISODateText(value) {
          LabeledContent(field.label + (field.kind == .date ? " (ISO Gregorian)" : " (UTC ISO)")) {
            TextField(field.kind == .date ? "YYYY-MM-DD" : "YYYY-MM-DDTHH:mm:00Z", text: Binding(
              get: { value.string },
              set: { value = field.kind == .date ? .date($0) : .dateTime($0) }))
              .textFieldStyle(.roundedBorder)
              .frame(minHeight: 44)
          }
          .onAppear { editingISODate = true }
        } else {
        DatePicker(field.kind == .dateTime ? field.label + " (UTC)" : field.label, selection: Binding(
          get: { NodAnswerField.calendarValue(value.string, kind: field.kind) ?? Date(timeIntervalSince1970: 0) },
          set: {
            let selected = NodAnswerField.pickerValue($0, kind:field.kind)
            if NodAnswerField.requiresISODateText(selected) { editingISODate = true }
            value = selected
          }), displayedComponents: field.kind == .date ? [.date] : [.date, .hourAndMinute])
          .datePickerStyle(.compact)
          .environment(\.calendar, NodAnswerField.pickerCalendar)
          .environment(\.timeZone, TimeZone(secondsFromGMT: 0)!)
        }
      }
    }
    .frame(minHeight: 44)
    .accessibilityIdentifier("answer-value")
  }
}

/// Presentation only: every choice still goes through the caller's Store path.
struct RequestAnswerButton: View {
  let option: NodRequestOption
  let isSubmitting: Bool
  let perform: (NodRequestOption) -> Void

  var body: some View {
    Button(role: option.destructive ? .destructive : nil) { perform(option) } label: {
      Text(option.label)
        .multilineTextAlignment(.leading)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }
    .buttonStyle(.bordered)
    .disabled(isSubmitting)
    .accessibilityIdentifier("answer-option-\(option.id)")
  }
}

/// A bounded reader, not an unbounded stack in a decision tray. Kept separate
/// from text composition so future protocol work need not replace this surface.
struct RequestAnswerOptions: View {
  let options: [NodRequestOption]
  let isSubmitting: Bool
  let perform: (NodRequestOption) -> Void
  @FocusState private var focusedOption: String?
  @State private var rememberedOption: String?
  var isComposing = false

  var body: some View {
    ScrollViewReader { reader in
      ScrollView {
        VStack(alignment: .leading, spacing: 8) {
          ForEach(options) { option in
            RequestAnswerButton(option: option, isSubmitting: isSubmitting || isComposing) { selected in
              guard !isComposing else { return }
              rememberedOption = selected.id
              focusedOption = selected.id
              perform(selected)
            }
              .id(option.id)
              .focused($focusedOption, equals: option.id)
          }
        }
        .padding()
      }
      .onChange(of: focusedOption) { _, id in
        if let id { rememberedOption = id; reader.scrollTo(id) }
      }
      .accessibilityIdentifier("answer-options")
      .onAppear { if !isComposing { focusedOption = rememberedOption ?? options.first?.id } }
      .onChange(of: isComposing) { _, composing in
        if composing { focusedOption = nil }
        else { focusedOption = rememberedOption ?? options.first?.id }
      }
    }
  }
}

struct RequestAnswerSheet: View {
  @Environment(\.dismiss) private var dismiss
  let options: [NodRequestOption]
  let isSubmitting: Bool
  let perform: (NodRequestOption) -> Void
  var isComposing = false

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text("Answer · \(options.count) options").font(.headline)
        Spacer()
        Button("Back to request") { dismiss() }
          .frame(minHeight: 44)
          .disabled(isComposing)
      }
      .padding(.horizontal)
      Divider()
      RequestAnswerOptions(options: options, isSubmitting: isSubmitting, perform: perform, isComposing: isComposing)
    }
  }
}

struct RequestDecisionTray: View {
  let options: [NodRequestOption]
  let isSubmitting: Bool
  let perform: (NodRequestOption) -> Void
  let expand: () -> Void

  var body: some View {
    ViewThatFits(in: .vertical) {
      if options.count <= 3 {
        VStack(spacing: 8) {
          ForEach(options) { option in
            RequestAnswerButton(option: option, isSubmitting: isSubmitting, perform: perform)
          }
        }
        .fixedSize(horizontal: false, vertical: true)
      }
      Button("Answer · \(options.count) options", action: expand)
        .frame(maxWidth: .infinity, minHeight: 44)
        .buttonStyle(.bordered)
        .disabled(isSubmitting)
        .accessibilityIdentifier("answer-access")
    }
    .padding()
    .background(.bar)
  }
}

struct RequestDestructiveAnswerConfirmation: View {
  let requestTitle: String
  let option: NodRequestOption
  var valueLabel: String? = nil
  let cancel: () -> Void
  let confirm: () -> Void

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 16) {
        Text(requestTitle).font(.headline)
        if let valueLabel { Text(valueLabel) }
        Text(option.label).fixedSize(horizontal: false, vertical: true)
        Button("Cancel", action: cancel).frame(minHeight: 44)
          .accessibilityIdentifier("cancel-destructive-answer")
        Button(role: .destructive, action: confirm) {
          Text(option.label).fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        }
        .buttonStyle(.bordered)
        .accessibilityIdentifier("confirm-destructive-answer")
      }.padding()
    }
  }
}
