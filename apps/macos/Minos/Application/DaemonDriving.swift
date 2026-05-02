import Foundation

/// Cancellation seam for daemon event subscriptions. Exists so test doubles
/// can satisfy `DaemonDriving` observer subscriptions without subclassing the
/// UniFFI-generated `Subscription` concrete type (which would require using
/// its private `noHandle` / `unsafeFromHandle` initializers).
protocol SubscriptionHandle: AnyObject, Sendable {
    func cancel()
}

/// The daemon surface AppState binds against. Mirrors the post-Phase-F
/// `DaemonHandle` UniFFI shape: dual-axis state (relay link + peer),
/// async pairing/forget round-trips, plus the multi-thread agent-runtime
/// methods that replaced the pre-Phase-C single-session surface
/// (`stop_agent` retired in favour of per-thread `interrupt_thread` /
/// `close_thread`). Tests use `MockDaemon` (Phase K.1) to satisfy this
/// protocol.
protocol DaemonDriving: AnyObject, Sendable {
    // ── Dual-axis state ──
    func currentRelayLink() -> RelayLinkState
    func currentPeer() -> PeerState
    func currentTrustedDevice() async throws -> PeerRecord?

    // ── Pairing round-trips ──
    func pairingQr() async throws -> RelayQrPayload
    func forgetPeer() async throws

    // ── Lifecycle ──
    func stop() async throws

    // ── Agent runtime (post-Phase-C multi-thread surface) ──
    func currentAgentState() -> ThreadState
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse
    func sendUserMessage(_ req: SendUserMessageRequest) async throws
    func interruptThread(_ req: InterruptThreadRequest) async throws
    func closeThread(_ req: CloseThreadRequest) async throws

    // ── Push-model observers ──
    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle
    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle
    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle
}
