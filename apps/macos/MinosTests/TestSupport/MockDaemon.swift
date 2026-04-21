import Foundation

@testable import Minos

final class MockSubscription: Subscription, @unchecked Sendable {
    private(set) var cancelCallCount = 0

    required init(unsafeFromHandle handle: UInt64) {
        super.init(unsafeFromHandle: handle)
    }

    init() {
        super.init(noHandle: .init())
    }

    override func cancel() {
        cancelCallCount += 1
    }
}

final class MockDaemon: DaemonDriving, @unchecked Sendable {
    var currentStateValue: ConnectionState
    var currentTrustedDeviceValue: TrustedDevice?
    var currentTrustedDeviceError: MinosError?
    var hostValue: String
    var pairingQrResult: Result<QrPayload, MinosError>
    var portValue: UInt16
    var forgetDeviceError: MinosError?
    var stopError: MinosError?

    let subscription: MockSubscription

    private(set) var forgetDeviceCalls: [DeviceId] = []
    private(set) var pairingQrCallCount = 0
    private(set) var stopCallCount = 0
    private(set) var subscribeCallCount = 0
    private(set) var observers: [ConnectionStateObserver] = []

    init(
        currentState: ConnectionState = .disconnected,
        currentTrustedDevice: TrustedDevice? = nil,
        host: String = "100.64.0.10",
        port: UInt16 = 7878,
        pairingQrResult: Result<QrPayload, MinosError> = .success(MockDaemon.makeQrPayload()),
        subscription: MockSubscription = MockSubscription()
    ) {
        currentStateValue = currentState
        currentTrustedDeviceValue = currentTrustedDevice
        hostValue = host
        self.pairingQrResult = pairingQrResult
        portValue = port
        self.subscription = subscription
    }

    func currentState() -> ConnectionState {
        currentStateValue
    }

    func currentTrustedDevice() throws -> TrustedDevice? {
        if let currentTrustedDeviceError {
            throw currentTrustedDeviceError
        }

        return currentTrustedDeviceValue
    }

    func forgetDevice(id: DeviceId) async throws {
        forgetDeviceCalls.append(id)
        if let forgetDeviceError {
            throw forgetDeviceError
        }
    }

    func host() -> String {
        hostValue
    }

    func pairingQr() throws -> QrPayload {
        pairingQrCallCount += 1
        return try pairingQrResult.get()
    }

    func port() -> UInt16 {
        portValue
    }

    func stop() async throws {
        stopCallCount += 1
        if let stopError {
            throw stopError
        }
    }

    func subscribe(observer: ConnectionStateObserver) -> Subscription {
        subscribeCallCount += 1
        observers.append(observer)
        return subscription
    }

    func emit(_ state: ConnectionState) {
        for observer in observers {
            observer.onState(state: state)
        }
    }

    static func makeQrPayload(
        host: String = "100.64.0.10",
        port: UInt16 = 7878,
        token: PairingToken = "pairing-token",
        name: String = "Minos Mac"
    ) -> QrPayload {
        QrPayload(v: 1, host: host, port: port, token: token, name: name)
    }

    static func makeTrustedDevice(
        deviceId: DeviceId = UUID().uuidString,
        name: String = "Alice's iPhone",
        host: String = "100.64.0.20",
        port: UInt16 = 7878,
        pairedAt: Date = Date(timeIntervalSince1970: 1_700_000_000)
    ) -> TrustedDevice {
        TrustedDevice(
            deviceId: deviceId,
            name: name,
            host: host,
            port: port,
            pairedAt: pairedAt
        )
    }
}
