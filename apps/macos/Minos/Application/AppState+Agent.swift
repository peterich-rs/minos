import Foundation
import OSLog

private let agentLogger = Logger(subsystem: "ai.minos.macos", category: "appState.agent")

/// Agent-axis methods on AppState. Lives in its own file so the core
/// AppState type body stays under the swiftlint type-body-length cap.
extension AppState {
    @MainActor
    func applyAgentState(_ state: AgentState) {
        agentState = state

        switch state {
        case .idle, .crashed:
            currentSession = nil
        case .starting, .running, .stopping:
            break
        }
    }

    @MainActor
    func startAgent(mode: AgentLaunchMode = .jsonl) async {
        guard phase == .running, let daemon else { return }

        clearAgentError()
        currentSession = nil

        let modeLabel = mode.logLabel
        agentLogger.info("startAgent requested · mode=\(modeLabel, privacy: .public)")

        do {
            let session = try await daemon.startAgent(.init(agent: .codex, mode: mode))
            currentSession = session
            agentLogger.info(
                "startAgent ok · mode=\(modeLabel, privacy: .public) · sessionId=\(session.sessionId, privacy: .public)"
            )
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            agentLogger.error(
                "startAgent failed · mode=\(modeLabel, privacy: .public) · \(error.technicalDetails, privacy: .public)"
            )
            presentAgentError(error)
        } catch {
            let detail = String(describing: error)
            agentLogger.error(
                "Unexpected start-agent error · mode=\(modeLabel, privacy: .public) · \(detail, privacy: .public)"
            )
        }
    }

    @MainActor
    func sendAgentPing() async {
        guard phase == .running, let daemon, let currentSession else { return }

        clearAgentError()
        agentLogger.info(
            "sendUserMessage(ping) · sessionId=\(currentSession.sessionId, privacy: .public)"
        )

        do {
            try await daemon.sendUserMessage(.init(sessionId: currentSession.sessionId, text: "ping"))
        } catch let error as MinosError {
            self.currentSession = nil
            agentState = .idle
            agentLogger.error("sendUserMessage(ping) failed · \(error.technicalDetails, privacy: .public)")
            presentAgentError(error)
        } catch {
            agentLogger.error("Unexpected agent ping error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func stopAgent() async {
        guard phase == .running, let daemon else { return }

        clearAgentError()
        agentLogger.info("stopAgent requested")

        do {
            try await daemon.stopAgent()
            currentSession = nil
            agentLogger.info("stopAgent ok")
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            agentLogger.error("stopAgent failed · \(error.technicalDetails, privacy: .public)")
            presentAgentError(error)
        } catch {
            agentLogger.error("Unexpected stop-agent error: \(String(describing: error), privacy: .public)")
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
        agentLogger.error("Presenting agent error: \(error.technicalDetails, privacy: .public)")
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
