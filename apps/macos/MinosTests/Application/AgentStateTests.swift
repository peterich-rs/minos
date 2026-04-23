import XCTest

@testable import Minos

final class AgentStateTests: XCTestCase {
    @MainActor
    func testBootstrapObserverDrivesAgentState() async {
        let daemon = MockDaemon()
        let appState = AppState(terminator: {})
        let expectedState = AgentState.running(
            agent: .codex,
            threadId: "thread-42",
            startedAt: Date(timeIntervalSince1970: 1_700_000_000)
        )

        await DaemonBootstrap.bootstrap(appState, startDaemon: { _ in daemon })
        daemon.emitAgentState(expectedState)
        await Task.yield()

        XCTAssertEqual(daemon.subscribeAgentStateCallCount, 1)
        XCTAssertEqual(appState.agentState, expectedState)
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

        XCTAssertEqual(daemon.startAgentCalls, [StartAgentRequest(agent: .codex)])
        XCTAssertEqual(appState.currentSession?.sessionId, "t1")
        XCTAssertNil(appState.agentError)
    }

    @MainActor
    func testStartAgentFailurePublishesAgentError() async {
        let daemon = MockDaemon(startAgentResult: .failure(.AgentAlreadyRunning))
        let appState = await bootedAppState(with: daemon)

        await appState.startAgent()

        XCTAssertEqual(daemon.startAgentCalls, [StartAgentRequest(agent: .codex)])
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
    func testStopAgentClearsCurrentSession() async {
        let daemon = MockDaemon()
        let appState = await bootedAppState(with: daemon)
        appState.currentSession = MockDaemon.makeStartAgentResponse(sessionId: "thread-99", cwd: "/w")

        await appState.stopAgent()

        XCTAssertEqual(daemon.stopAgentCallCount, 1)
        XCTAssertNil(appState.currentSession)
    }

    @MainActor
    func testDismissAgentCrashClearsErrorButKeepsState() {
        let appState = AppState()
        appState.agentState = .crashed(reason: "exit code 137")
        appState.agentError = .CodexProtocolError(method: "turn/start", message: "boom")

        appState.dismissAgentCrash()

        XCTAssertNil(appState.agentError)
        XCTAssertEqual(appState.agentState, .crashed(reason: "exit code 137"))
    }

    @MainActor
    func testShutdownCancelsAgentSubscription() async {
        let daemon = MockDaemon(currentState: .connected)
        let connectionSubscription = MockSubscription()
        let agentSubscription = MockSubscription()
        let appState = AppState(terminator: {})

        appState.daemon = daemon
        appState.subscription = connectionSubscription
        appState.agentSubscription = agentSubscription

        await appState.shutdown()

        XCTAssertEqual(agentSubscription.cancelCallCount, 1)
    }

    @MainActor
    private func bootedAppState(with daemon: MockDaemon) async -> AppState {
        let appState = AppState(terminator: {})
        await DaemonBootstrap.bootstrap(appState, startDaemon: { _ in daemon })
        return appState
    }
}
