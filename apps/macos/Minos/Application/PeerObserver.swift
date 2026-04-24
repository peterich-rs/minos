import Foundation

/// Symmetric to `RelayLinkObserver` for the peer-pairing axis. Forwards
/// every push from the Rust dispatcher to a closure the AppState owns.
///
/// `@unchecked Sendable` for the same reason — see RelayLinkObserver.
final class PeerObserver: PeerStateObserver, @unchecked Sendable {
    private let onStateChange: @Sendable (PeerState) -> Void

    init(onStateChange: @escaping @Sendable (PeerState) -> Void) {
        self.onStateChange = onStateChange
    }

    func onState(state: PeerState) {
        onStateChange(state)
    }
}
