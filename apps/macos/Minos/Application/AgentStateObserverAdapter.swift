import Foundation

final class AgentStateObserverAdapter: AgentStateObserver, @unchecked Sendable {
    private let onUpdate: @Sendable (AgentState) -> Void

    init(onUpdate: @escaping @Sendable (AgentState) -> Void) {
        self.onUpdate = onUpdate
    }

    func onState(state: AgentState) {
        onUpdate(state)
    }
}
