import Foundation

/// UI validation only. The runtime remains authoritative for every answer.
public enum NodAnswerComposition {
  public static func notesKey(serverId: String, requestId: String, optionId: String, isValueRequest: Bool) -> String {
    [serverId, requestId, isValueRequest ? "value-notes" : optionId].joined(separator: ":")
  }
  public static func canSend(text: String, isSubmitting: Bool) -> Bool {
    !isSubmitting && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }
}
