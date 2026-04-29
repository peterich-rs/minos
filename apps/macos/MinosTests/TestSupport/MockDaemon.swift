import Foundation

@testable import Minos

final class MockSubscription: SubscriptionHandle, @unchecked Sendable {
    private(set) var cancelCallCount = 0

    func cancel() {
        cancelCallCount += 1
    }
}

/// Test double conforming to the Phase F.1 dual-axis `DaemonDriving`
/// protocol. Replaces the legacy single-axis MockDaemon — both axes are
/// now exposed independently and observers can be fired from either
/// channel without affecting the other.
///
/// Plan 05 Phase K.1.
final class MockDaemon: DaemonDriving, @unchecked Sendable {
    // ── Public mutable state ──
    var currentRelayLinkValue: RelayLinkState
    var currentPeerValue: PeerState
    var currentAgentStateValue: AgentState
    var currentTrustedDeviceValue: PeerRecord?
    var currentTrustedDeviceError: MinosError?
    var pairingQrResult: Result<RelayQrPayload, MinosError>
    var forgetPeerError: MinosError?
    var startAgentResult: Result<StartAgentResponse, MinosError>
    var sendUserMessageError: MinosError?
    var stopAgentError: MinosError?
    var stopError: MinosError?

    let relayLinkSubscription: MockSubscription
    let peerSubscription: MockSubscription
    let agentSubscription: MockSubscription

    private(set) var forgetPeerCallCount = 0
    private(set) var pairingQrCallCount = 0
    private(set) var startAgentCalls: [StartAgentRequest] = []
    private(set) var sendUserMessageCalls: [SendUserMessageRequest] = []
    private(set) var stopAgentCallCount = 0
    private(set) var stopCallCount = 0
    private(set) var subscribeRelayLinkCallCount = 0
    private(set) var subscribePeerCallCount = 0
    private(set) var subscribeAgentStateCallCount = 0
    private(set) var relayLinkObservers: [RelayLinkStateObserver] = []
    private(set) var peerObservers: [PeerStateObserver] = []
    private(set) var agentObservers: [AgentStateObserver] = []

    init(
        currentRelayLink: RelayLinkState = .disconnected,
        currentPeer: PeerState = .unpaired,
        currentAgentState: AgentState = .idle,
        currentTrustedDevice: PeerRecord? = nil,
        pairingQrResult: Result<RelayQrPayload, MinosError> = .success(MockDaemon.makeQrPayload()),
        startAgentResult: Result<StartAgentResponse, MinosError> = .success(
            MockDaemon.makeStartAgentResponse()
        ),
        relayLinkSubscription: MockSubscription = MockSubscription(),
        peerSubscription: MockSubscription = MockSubscription(),
        agentSubscription: MockSubscription = MockSubscription()
    ) {
        currentRelayLinkValue = currentRelayLink
        currentPeerValue = currentPeer
        currentAgentStateValue = currentAgentState
        currentTrustedDeviceValue = currentTrustedDevice
        self.pairingQrResult = pairingQrResult
        self.startAgentResult = startAgentResult
        self.relayLinkSubscription = relayLinkSubscription
        self.peerSubscription = peerSubscription
        self.agentSubscription = agentSubscription
    }

    // ── DaemonDriving ──

    func currentRelayLink() -> RelayLinkState { currentRelayLinkValue }
    func currentPeer() -> PeerState { currentPeerValue }
    func currentAgentState() -> AgentState { currentAgentStateValue }

    func currentTrustedDevice() async throws -> PeerRecord? {
        if let currentTrustedDeviceError {
            throw currentTrustedDeviceError
        }
        return currentTrustedDeviceValue
    }

    func pairingQr() async throws -> RelayQrPayload {
        pairingQrCallCount += 1
        return try pairingQrResult.get()
    }

    func forgetPeer() async throws {
        forgetPeerCallCount += 1
        if let forgetPeerError {
            throw forgetPeerError
        }
        // Mirror the relay's behaviour: a successful ForgetPeer pushes an
        // Unpaired event to the peer observer shortly after. Tests can
        // pre-empt this by setting `currentPeerValue = .unpaired` directly.
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

    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle {
        subscribeRelayLinkCallCount += 1
        relayLinkObservers.append(observer)
        return relayLinkSubscription
    }

    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle {
        subscribePeerCallCount += 1
        peerObservers.append(observer)
        return peerSubscription
    }

    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle {
        subscribeAgentStateCallCount += 1
        agentObservers.append(observer)
        return agentSubscription
    }

    // ── Test helpers ──

    /// Push a fresh relay-link state to all subscribed observers and
    /// update the snapshot value.
    func emitRelayLink(_ state: RelayLinkState) {
        currentRelayLinkValue = state
        for observer in relayLinkObservers {
            observer.onState(state: state)
        }
    }

    /// Push a fresh peer state to all subscribed observers and update the
    /// snapshot value.
    func emitPeer(_ state: PeerState) {
        currentPeerValue = state
        for observer in peerObservers {
            observer.onState(state: state)
        }
    }

    func emitAgentState(_ state: AgentState) {
        currentAgentStateValue = state
        for observer in agentObservers {
            observer.onState(state: state)
        }
    }

    // ── Convenience factories ──

    static func makeQrPayload(
        pairingToken: PairingToken = "pairing-token",
        hostDisplayName: String = "Minos Mac",
        expiresAtMs: Int64 = 1_700_000_000_000
    ) -> RelayQrPayload {
        RelayQrPayload(
            v: 2,
            hostDisplayName: hostDisplayName,
            pairingToken: pairingToken,
            expiresAtMs: expiresAtMs
        )
    }

    static func makeStartAgentResponse(
        sessionId: String = "thread-abc12",
        cwd: String = "/Users/fan/.minos/workspaces"
    ) -> StartAgentResponse {
        StartAgentResponse(sessionId: sessionId, cwd: cwd)
    }

    static func makeTrustedDevice(
        deviceId: DeviceId = UUID().uuidString.lowercased(),
        name: String = "Alice's iPhone",
        pairedAt: Date = Date(timeIntervalSince1970: 1_700_000_000)
    ) -> PeerRecord {
        PeerRecord(deviceId: deviceId, name: name, pairedAt: pairedAt)
    }
}
