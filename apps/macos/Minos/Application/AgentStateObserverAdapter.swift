import Foundation

final class AgentStateObserverAdapter: AgentStateObserver, @unchecked Sendable {
    private let onUpdate: @Sendable (ThreadState) -> Void

    init(onUpdate: @escaping @Sendable (ThreadState) -> Void) {
        self.onUpdate = onUpdate
    }

    func onState(state: ThreadState) {
        onUpdate(state)
    }
}
