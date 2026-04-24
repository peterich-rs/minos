import Foundation

/// Cancellation seam for daemon event subscriptions. Exists so test doubles
/// can satisfy `DaemonDriving` observer subscriptions without subclassing the
/// UniFFI-generated `Subscription` concrete type (which would require using
/// its private `noHandle` / `unsafeFromHandle` initializers).
protocol SubscriptionHandle: AnyObject, Sendable {
    func cancel()
}

protocol DaemonDriving: AnyObject, Sendable {
    func currentState() -> ConnectionState
    func currentTrustedDevice() throws -> TrustedDevice?
    func forgetDevice(id: DeviceId) async throws
    func host() -> String
    func pairingQr() throws -> QrPayload
    func port() -> UInt16
    func currentAgentState() -> AgentState
    func stop() async throws
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse
    func sendUserMessage(_ req: SendUserMessageRequest) async throws
    func stopAgent() async throws
    func subscribeObserver(_ observer: ConnectionStateObserver) -> any SubscriptionHandle
    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle
}
