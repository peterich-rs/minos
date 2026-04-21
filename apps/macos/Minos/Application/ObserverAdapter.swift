import Foundation

final class ObserverAdapter: ConnectionStateObserver, @unchecked Sendable {
    private let onStateChange: @Sendable (ConnectionState) -> Void

    init(onStateChange: @escaping @Sendable (ConnectionState) -> Void) {
        self.onStateChange = onStateChange
    }

    func onState(state: ConnectionState) {
        onStateChange(state)
    }
}
