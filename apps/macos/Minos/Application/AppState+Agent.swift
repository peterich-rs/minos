import Foundation

/// Agent-axis methods on AppState. Lives in its own file so the core
/// AppState type body stays under the swiftlint type-body-length cap.
extension AppState {
    @MainActor
    func applyAgentState(_ state: ThreadState) {
        agentState = state

        // Post-Phase-C the daemon's legacy single-channel state mirror always
        // pushes `.idle` after a successful `start_agent` / `close_thread`
        // ("the multi-thread manager keeps per-thread state internally; the
        // single-channel mirror just signals 'something is alive'", agent.rs).
        // So `.idle` no longer means "no session" — it means "thread alive,
        // not in a turn". The session lifecycle is now driven explicitly by
        // `startAgent` / `stopAgent` below, which set / clear `currentSession`
        // around the round-trip. Only `.closed` warrants clearing here, and
        // even that is mostly defensive — the legacy mirror does not emit
        // `.closed` today.
        if case .closed = state {
            currentSession = nil
        }
    }

    @MainActor
    func startAgent(mode: AgentLaunchMode = .jsonl) async {
        guard phase == .running, let daemon else { return }

        clearAgentError()
        currentSession = nil

        let modeLabel = mode.logLabel
        AppLog.info("appState.agent", "startAgent requested · mode=\(modeLabel)")

        do {
            let session = try await daemon.startAgent(
                .init(agent: .codex, workspace: "", mode: mode)
            )
            currentSession = session
            AppLog.info("appState.agent", "startAgent ok · mode=\(modeLabel) · sessionId=\(session.sessionId)")
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            AppLog.error("appState.agent", "startAgent failed · mode=\(modeLabel) · \(error.technicalDetails)")
            presentAgentError(error)
        } catch {
            let detail = String(describing: error)
            AppLog.error("appState.agent", "Unexpected start-agent error · mode=\(modeLabel) · \(detail)")
        }
    }

    @MainActor
    func sendAgentPing() async {
        guard phase == .running, let daemon, let currentSession else { return }

        clearAgentError()
        AppLog.info("appState.agent", "sendUserMessage(ping) · sessionId=\(currentSession.sessionId)")

        do {
            try await daemon.sendUserMessage(.init(sessionId: currentSession.sessionId, text: "ping"))
        } catch let error as MinosError {
            self.currentSession = nil
            agentState = .idle
            AppLog.error("appState.agent", "sendUserMessage(ping) failed · \(error.technicalDetails)")
            presentAgentError(error)
        } catch {
            AppLog.error("appState.agent", "Unexpected agent ping error: \(String(describing: error))")
        }
    }

    /// Per-thread "stop" — translates the legacy single-session "Stop"
    /// affordance to the new `close_thread` RPC keyed by the live
    /// `currentSession.sessionId`. With no live session there is no thread
    /// to close, so the call is a no-op.
    @MainActor
    func stopAgent() async {
        guard phase == .running, let daemon, let session = currentSession else { return }

        clearAgentError()
        let threadId = session.sessionId
        AppLog.info("appState.agent", "stopAgent requested · threadId=\(threadId)")

        do {
            try await daemon.closeThread(.init(threadId: threadId))
            currentSession = nil
            AppLog.info("appState.agent", "stopAgent ok")
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            AppLog.error("appState.agent", "stopAgent failed · \(error.technicalDetails)")
            presentAgentError(error)
        } catch {
            AppLog.error("appState.agent", "Unexpected stop-agent error: \(String(describing: error))")
        }
    }

    @MainActor
    func dismissAgentCrash() {
        clearAgentError()
    }

    @MainActor
    func clearAgentError() {
        agentErrorTask?.cancel()
        agentError = nil
    }

    @MainActor
    func presentAgentError(_ error: MinosError) {
        AppLog.error("appState.agent", "Presenting agent error: \(error.technicalDetails)")
        agentErrorTask?.cancel()
        agentError = error
        agentErrorTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            await MainActor.run {
                self?.agentError = nil
            }
        }
    }
}

extension AgentLaunchMode {
    /// Stable, ASCII-only label that matches the Rust-side `tracing` field —
    /// pairing the two streams together when reading logs side-by-side.
    var logLabel: String {
        switch self {
        case .jsonl: return "jsonl"
        case .server: return "server"
        }
    }
}
