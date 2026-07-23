import Foundation

final class AgentStateObserverAdapter: AgentStateObserver, @unchecked Sendable {
    private let onUpdate: @Sendable (SessionState) -> Void

    init(onUpdate: @escaping @Sendable (SessionState) -> Void) {
        self.onUpdate = onUpdate
    }

    func onState(state: SessionState) {
        onUpdate(state)
    }
}
