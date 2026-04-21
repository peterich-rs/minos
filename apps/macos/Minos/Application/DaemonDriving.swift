import Foundation

protocol DaemonDriving: AnyObject, Sendable {
    func currentState() -> ConnectionState
    func currentTrustedDevice() throws -> TrustedDevice?
    func forgetDevice(id: DeviceId) async throws
    func host() -> String
    func pairingQr() throws -> QrPayload
    func port() -> UInt16
    func stop() async throws
    func subscribe(observer: ConnectionStateObserver) -> Subscription
}
