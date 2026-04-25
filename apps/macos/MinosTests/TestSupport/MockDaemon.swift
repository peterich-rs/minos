import Foundation

@testable import Minos

final class MockSubscription: SubscriptionHandle, @unchecked Sendable {
    private(set) var cancelCallCount = 0

    func cancel() {
        cancelCallCount += 1
    }
}

final class MockDaemon: DaemonDriving, @unchecked Sendable {
    var currentStateValue: ConnectionState
    var currentAgentStateValue: AgentState
    var currentTrustedDeviceValue: TrustedDevice?
    var currentTrustedDeviceError: MinosError?
    var hostValue: String
    var pairingQrResult: Result<QrPayload, MinosError>
    var portValue: UInt16
    var forgetDeviceError: MinosError?
    var startAgentResult: Result<StartAgentResponse, MinosError>
    var sendUserMessageError: MinosError?
    var stopAgentError: MinosError?
    var stopError: MinosError?

    let subscription: MockSubscription
    let agentSubscription: MockSubscription

    private(set) var forgetDeviceCalls: [DeviceId] = []
    private(set) var pairingQrCallCount = 0
    private(set) var startAgentCalls: [StartAgentRequest] = []
    private(set) var sendUserMessageCalls: [SendUserMessageRequest] = []
    private(set) var stopAgentCallCount = 0
    private(set) var stopCallCount = 0
    private(set) var subscribeCallCount = 0
    private(set) var subscribeAgentStateCallCount = 0
    private(set) var observers: [ConnectionStateObserver] = []
    private(set) var agentObservers: [AgentStateObserver] = []

    init(
        currentState: ConnectionState = .disconnected,
        currentAgentState: AgentState = .idle,
        currentTrustedDevice: TrustedDevice? = nil,
        host: String = "100.64.0.10",
        port: UInt16 = 7878,
        pairingQrResult: Result<QrPayload, MinosError> = .success(MockDaemon.makeQrPayload()),
        startAgentResult: Result<StartAgentResponse, MinosError> = .success(
            MockDaemon.makeStartAgentResponse()
        ),
        subscription: MockSubscription = MockSubscription(),
        agentSubscription: MockSubscription = MockSubscription()
    ) {
        currentStateValue = currentState
        currentAgentStateValue = currentAgentState
        currentTrustedDeviceValue = currentTrustedDevice
        hostValue = host
        self.pairingQrResult = pairingQrResult
        portValue = port
        self.startAgentResult = startAgentResult
        self.subscription = subscription
        self.agentSubscription = agentSubscription
    }

    func currentState() -> ConnectionState {
        currentStateValue
    }

    func currentAgentState() -> AgentState {
        currentAgentStateValue
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

    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse {
        startAgentCalls.append(req)
        return try startAgentResult.get()
    }

    func sendUserMessage(_ req: SendUserMessageRequest) async throws {
        sendUserMessageCalls.append(req)
        if let sendUserMessageError {
            throw sendUserMessageError
        }
    }

    func stopAgent() async throws {
        stopAgentCallCount += 1
        if let stopAgentError {
            throw stopAgentError
        }
    }

    func subscribeObserver(_ observer: ConnectionStateObserver) -> any SubscriptionHandle {
        subscribeCallCount += 1
        observers.append(observer)
        return subscription
    }

    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle {
        subscribeAgentStateCallCount += 1
        agentObservers.append(observer)
        return agentSubscription
    }

    func emit(_ state: ConnectionState) {
        currentStateValue = state
        for observer in observers {
            observer.onState(state: state)
        }
    }

    func emitAgentState(_ state: AgentState) {
        currentAgentStateValue = state
        for observer in agentObservers {
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

    static func makeStartAgentResponse(
        sessionId: String = "thread-abc12",
        cwd: String = "/Users/fan/.minos/workspaces"
    ) -> StartAgentResponse {
        StartAgentResponse(sessionId: sessionId, cwd: cwd)
    }

    static func makeTrustedDevice(
        deviceId: DeviceId = UUID().uuidString,
        name: String = "Alice's iPhone",
        hostDeviceId: DeviceId? = nil,
        host: String = "100.64.0.20",
        port: UInt16 = 7878,
        assignedDeviceSecret: DeviceSecret? = nil,
        pairedAt: Date = Date(timeIntervalSince1970: 1_700_000_000)
    ) -> TrustedDevice {
        TrustedDevice(
            deviceId: deviceId,
            name: name,
            hostDeviceId: hostDeviceId,
            host: host,
            port: port,
            assignedDeviceSecret: assignedDeviceSecret,
            pairedAt: pairedAt
        )
    }
}
