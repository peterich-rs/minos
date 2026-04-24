import Foundation

/// Swift-side adapter that conforms to the Rust-generated
/// `RelayLinkStateObserver` callback protocol and forwards each push to a
/// closure. `AppState` owns the closure capture so the observer can
/// safely outlive the menu view that subscribed.
///
/// `@unchecked Sendable` because Rust hands the callback to its own
/// background dispatcher; the closure target is the Tokio runtime, not
/// the main actor. Bridging back to `@MainActor` is the closure's
/// responsibility (see `DaemonBootstrap`).
final class RelayLinkObserver: RelayLinkStateObserver, @unchecked Sendable {
    private let onStateChange: @Sendable (RelayLinkState) -> Void

    init(onStateChange: @escaping @Sendable (RelayLinkState) -> Void) {
        self.onStateChange = onStateChange
    }

    func onState(state: RelayLinkState) {
        onStateChange(state)
    }
}
