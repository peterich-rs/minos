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
/// async pairing/forget round-trips, plus the multi-session agent-runtime
/// methods that replaced the pre-Phase-C single-session surface
/// (`stop_agent` retired in favour of per-session `interrupt_session` /
/// `close_session`). Tests use `MockDaemon` (Phase K.1) to satisfy this
/// protocol.
protocol DaemonDriving: AnyObject, Sendable {
    // ── Dual-axis state ──
    func currentRelayLink() -> RelayLinkState
    func currentPeer() -> PeerState
    func currentTrustedDevice() async throws -> PeerRecord?
    func currentPeers() async throws -> [HostPeerSummary]

    // ── Pairing round-trips ──
    func pairingQr() async throws -> RelayQrPayload
    func forgetPeer() async throws
    func forgetPeerDevice(_ mobileDeviceId: DeviceId) async throws

    // ── Lifecycle ──
    func stop() async throws

    // ── Agent runtime (multi-session surface) ──
    func currentAgentState() -> SessionState
    func currentAgentSession() async throws -> AgentSessionSnapshot?
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse
    func sendUserMessage(_ req: SendUserMessageRequest) async throws
    func interruptSession(_ req: InterruptSessionRequest) async throws
    func closeSession(_ req: CloseSessionRequest) async throws

    // ── Push-model observers ──
    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle
    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle
    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle
}
