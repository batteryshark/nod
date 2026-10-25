import Foundation

/// The same allowlist is used by the sidebar menu and its click-time guard.
public enum NodSidebarActions {
  public static func options(for request: NodRequest, userId: String?, now: Date = Date()) -> [NodRequestOption] {
    guard request.status == .pending, !request.isValueRequest,
      request.expiresAt.map({ $0 > now }) ?? true else { return [] }
    if request.decisionResolution == .shared {
      guard request.decision == nil else { return [] }
    } else {
      guard let userId, !request.decisions.contains(where: { $0.userId == userId }) else { return [] }
    }
    if request.isPendingInformational {
      return [NodRequestOption(id: "dismiss", label: "Acknowledge", kind: .dismiss)]
    }
    return request.options.filter {
      !$0.destructive && $0.style != "destructive" && !$0.requiresText
        && $0.kind != .approveWithText && $0.kind != .rejectWithText && $0.kind != .open
    }
  }
}

extension NodStore {
  public func sidebarActions(for request: NodRequest, serverId: String) -> [NodRequestOption] {
    guard runtime.state?.selectedServerId == serverId else { return [] }
    return NodSidebarActions.options(for: request, userId: runtime.state?.currentUser?.id)
  }

  public func canPerformSidebarAction(_ option: NodRequestOption, request: NodRequest, serverId: String) -> Bool {
    guard let state = runtime.state, state.selectedServerId == serverId,
      let digest = request.signing?.requestDigest, !digest.isEmpty,
      let latest = state.requests.first(where: { $0.id == request.id }),
      latest.signing?.requestDigest == digest,
      answerAttempt(requestId: request.id, serverId: serverId) == nil,
      !isSubmitting(request.id) else { return false }
    return sidebarActions(for: request, serverId: serverId).contains(option)
      && sidebarActions(for: latest, serverId: serverId).contains(option)
  }

  @discardableResult
  public func performSidebarAction(_ option: NodRequestOption, request: NodRequest, serverId: String) async -> Bool {
    guard canPerformSidebarAction(option, request: request, serverId: serverId) else { return false }
    // answerFromUI reserves the attempt synchronously, uses submit(request:...),
    // and reconciles with GET. An uncertain write is never automatically retried.
    return await answerFromUI(request: request, option: option, serverId: serverId)
  }
}
