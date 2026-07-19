import SwiftUI

struct AgentSegmentView: View {
    @Bindable var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Divider()

            stateRow

            if let agentError = appState.agentError {
                errorBanner(agentError)
            }

            #if DEBUG
            debugControls
            #endif

            Divider()
        }
    }

    @ViewBuilder
    private var stateRow: some View {
        // When a session is active, prefer rendering its identity + the
        // current state. The legacy single-channel state mirror only emits
        // `.idle` post-Phase-C, so gating session info on the `.running`
        // case (as the pre-Phase-C UI did) leaves the menubar looking
        // identical before and after `start_agent`. See
        // `AppState+Agent.applyAgentState` for why `.idle` no longer means
        // "no session".
        if let session = appState.currentSession {
            sessionRow(session)
        } else {
            bareStateRow
        }
    }

    @ViewBuilder
    private func sessionRow(_ session: StartAgentResponse) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Agent · \(stateLabel(appState.agentState))")
                .font(.subheadline.weight(.medium))
                .frame(maxWidth: .infinity, alignment: .leading)

            HStack(spacing: 6) {
                Text("Thread")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Text(session.sessionId)
                    .font(.caption.monospaced())
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            HStack(spacing: 6) {
                Text("Workspace")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Text(session.cwd)
                    .font(.caption2)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private var bareStateRow: some View {
        switch appState.agentState {
        case .idle:
            stateText("Agent: Idle")
        case .starting:
            stateText("Agent: Starting...")
        case .running:
            stateText("Agent: Running...")
        case let .suspended(reason):
            stateText("Agent: Suspended · \(label(for: reason))")
        case .resuming:
            stateText("Agent: Resuming...")
        case let .closed(reason):
            stateText("Agent: Closed · \(label(for: reason))")
        }
    }

    private func stateLabel(_ state: ThreadState) -> String {
        switch state {
        case .idle: return "Idle"
        case .starting: return "Starting"
        case .running: return "Running"
        case let .suspended(reason): return "Suspended (\(label(for: reason)))"
        case .resuming: return "Resuming"
        case let .closed(reason): return "Closed (\(label(for: reason)))"
        }
    }

    #if DEBUG
    @ViewBuilder
    private var debugControls: some View {
        // Gate the controls on `currentSession` rather than the legacy state
        // value: the daemon's single-channel state mirror only emits `.idle`
        // post-Phase-C, so a `switch appState.agentState` would always show
        // the start buttons even with a live thread.
        if appState.currentSession != nil {
            HStack(spacing: 8) {
                Button("发送 ping（测试）") {
                    Task {
                        await appState.sendAgentPing()
                    }
                }

                Button("停止 Codex", role: .destructive) {
                    Task {
                        await appState.stopAgent()
                    }
                }
            }
            .buttonStyle(.bordered)
        } else {
            switch appState.agentState {
            case .starting, .resuming:
                EmptyView()
            case .idle, .running, .suspended, .closed:
                HStack(spacing: 8) {
                    Button("启动 Codex (jsonl)") {
                        Task {
                            await appState.startAgent(mode: .jsonl)
                        }
                    }
                    Button("启动 Codex (server)") {
                        Task {
                            await appState.startAgent(mode: .server)
                        }
                    }
                }
                .buttonStyle(.bordered)
            }
        }
    }
    #endif

    private func label(for reason: PauseReason) -> String {
        switch reason {
        case .userInterrupt: return "user interrupt"
        case .codexCrashed: return "codex crashed"
        case .daemonRestart: return "daemon restart"
        case .instanceReaped: return "instance reaped"
        }
    }

    private func label(for reason: CloseReason) -> String {
        switch reason {
        case .userClose: return "user close"
        case .terminalError: return "terminal error"
        }
    }

    private func stateText(_ text: String) -> some View {
        Text(text)
            .font(.subheadline.weight(.medium))
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func errorBanner(_ error: MinosError) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)

            Text(error.userMessage(lang: .zh))
                .font(.caption)
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.orange.opacity(0.14))
        )
    }

}
