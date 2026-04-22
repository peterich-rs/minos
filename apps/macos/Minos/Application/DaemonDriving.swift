import Foundation

/// Cancellation seam for daemon event subscriptions. Exists so test doubles
/// can satisfy `DaemonDriving.subscribeObserver` without subclassing the
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
    func stop() async throws
    func subscribeObserver(_ observer: ConnectionStateObserver) -> any SubscriptionHandle
}
