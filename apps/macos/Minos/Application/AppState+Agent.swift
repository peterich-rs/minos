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
    func startAgent() async {
        guard phase == .running, let daemon else { return }

        clearAgentError()
        currentSession = nil

        do {
            currentSession = try await daemon.startAgent(.init(agent: .codex))
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            presentAgentError(error)
        } catch {
            agentLogger.error("Unexpected start-agent error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func sendAgentPing() async {
        guard phase == .running, let daemon, let currentSession else { return }

        clearAgentError()

        do {
            try await daemon.sendUserMessage(.init(sessionId: currentSession.sessionId, text: "ping"))
        } catch let error as MinosError {
            self.currentSession = nil
            agentState = .idle
            presentAgentError(error)
        } catch {
            agentLogger.error("Unexpected agent ping error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func stopAgent() async {
        guard phase == .running, let daemon else { return }

        clearAgentError()

        do {
            try await daemon.stopAgent()
            currentSession = nil
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
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
