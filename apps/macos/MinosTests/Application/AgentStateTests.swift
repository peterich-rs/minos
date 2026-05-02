import XCTest

@testable import Minos

/// Plan 05 Phase K.2: agent-axis tests rewritten to wire the
/// AgentStateObserverAdapter manually rather than driving through
/// DaemonBootstrap (which touches process env, Keychain, and the Rust
/// runtime — side effects we don't want in unit tests).
final class AgentStateTests: XCTestCase {
    @MainActor
    func testAgentObserverPushDrivesAppState() async {
        let daemon = MockDaemon()
        let appState = await bootedAppState(with: daemon)
        let expected = ThreadState.running(turnStartedAtMs: 1_700_000_000_000)

        daemon.emitAgentState(expected)
        await Task.yield()

        XCTAssertEqual(daemon.subscribeAgentStateCallCount, 1)
        XCTAssertEqual(appState.agentState, expected)
    }

    @MainActor
    func testStartAgentStoresSession() async {
        let daemon = MockDaemon(
            startAgentResult: .success(
                MockDaemon.makeStartAgentResponse(sessionId: "t1", cwd: "/w")
            )
        )
        let appState = await bootedAppState(with: daemon)

        await appState.startAgent()

        XCTAssertEqual(
            daemon.startAgentCalls,
            [StartAgentRequest(agent: .codex, workspace: "", mode: .jsonl)]
        )
        XCTAssertEqual(appState.currentSession?.sessionId, "t1")
        XCTAssertNil(appState.agentError)
    }

    @MainActor
    func testStartAgentServerModeForwardsModeToDaemon() async {
        let daemon = MockDaemon(
            startAgentResult: .success(
                MockDaemon.makeStartAgentResponse(sessionId: "t1", cwd: "/w")
            )
        )
        let appState = await bootedAppState(with: daemon)

        await appState.startAgent(mode: .server)

        XCTAssertEqual(
            daemon.startAgentCalls,
            [StartAgentRequest(agent: .codex, workspace: "", mode: .server)]
        )
    }

    @MainActor
    func testStartAgentFailurePublishesAgentError() async {
        let daemon = MockDaemon(startAgentResult: .failure(.AgentAlreadyRunning))
        let appState = await bootedAppState(with: daemon)

        await appState.startAgent()

        XCTAssertEqual(
            daemon.startAgentCalls,
            [StartAgentRequest(agent: .codex, workspace: "", mode: .jsonl)]
        )
        XCTAssertEqual(appState.agentState, .idle)
        XCTAssertEqual(appState.agentError, .AgentAlreadyRunning)
    }

    @MainActor
    func testSendAgentPingUsesCurrentSessionId() async {
        let daemon = MockDaemon()
        let appState = await bootedAppState(with: daemon)
        appState.currentSession = MockDaemon.makeStartAgentResponse(sessionId: "thread-99", cwd: "/w")

        await appState.sendAgentPing()

        XCTAssertEqual(
            daemon.sendUserMessageCalls,
            [SendUserMessageRequest(sessionId: "thread-99", text: "ping")]
        )
    }

    @MainActor
    func testStopAgentClosesActiveThread() async {
        let daemon = MockDaemon()
        let appState = await bootedAppState(with: daemon)
        appState.currentSession = MockDaemon.makeStartAgentResponse(sessionId: "thread-99", cwd: "/w")

        await appState.stopAgent()

        XCTAssertEqual(
            daemon.closeThreadCalls,
            [CloseThreadRequest(threadId: "thread-99")]
        )
        XCTAssertNil(appState.currentSession)
    }

    @MainActor
    func testDismissAgentCrashClearsErrorButKeepsState() {
        let appState = AppState()
        appState.agentState = .closed(reason: .terminalError)
        appState.agentError = .CodexProtocolError(method: "turn/start", message: "boom")

        appState.dismissAgentCrash()

        XCTAssertNil(appState.agentError)
        XCTAssertEqual(appState.agentState, .closed(reason: .terminalError))
    }

    @MainActor
    func testShutdownCancelsAgentSubscription() async {
        let daemon = MockDaemon(currentRelayLink: .connected, currentPeer: .unpaired)
        let relayLinkSub = MockSubscription()
        let peerSub = MockSubscription()
        let agentSub = MockSubscription()
        let appState = AppState(terminator: {})

        appState.daemon = daemon
        appState.relayLinkSubscription = relayLinkSub
        appState.peerSubscription = peerSub
        appState.agentSubscription = agentSub

        await appState.shutdown()

        XCTAssertEqual(agentSub.cancelCallCount, 1)
        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
    }

    /// Stand up an AppState in `.running` with the agent observer wired
    /// through MockDaemon. Skips DaemonBootstrap (which would need CF
    /// credentials we can't set without polluting the process env).
    @MainActor
    private func bootedAppState(with daemon: MockDaemon) async -> AppState {
        let appState = AppState(terminator: {})
        let agentObserver = AgentStateObserverAdapter { state in
            Task { @MainActor in appState.applyAgentState(state) }
        }
        let agentSub = daemon.subscribeAgentState(agentObserver)

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil,
            agentSubscription: agentSub
        )
        return appState
    }
}
