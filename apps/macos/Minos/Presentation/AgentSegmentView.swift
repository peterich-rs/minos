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
        switch appState.agentState {
        case .idle:
            stateText("Agent: Idle")
        case .starting:
            stateText("Agent: Starting...")
        case let .running(_, threadId, startedAt):
            TimelineView(.periodic(from: startedAt, by: 1)) { context in
                stateText(
                    "Agent: Running · thread \(threadId) · \(uptimeLabel(now: context.date, startedAt: startedAt))"
                )
            }
        case .stopping:
            stateText("Agent: Stopping...")
        case let .crashed(reason):
            stateText("Agent: Crashed · \(reason)")
        }
    }

    #if DEBUG
    @ViewBuilder
    private var debugControls: some View {
        switch appState.agentState {
        case .idle:
            Button("启动 Codex（测试）") {
                Task {
                    await appState.startAgent()
                }
            }
            .buttonStyle(.bordered)
        case .starting, .stopping:
            EmptyView()
        case .running:
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
        case .crashed:
            Button("关闭提示") {
                appState.dismissAgentCrash()
            }
            .buttonStyle(.bordered)
        }
    }
    #endif

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

    private func uptimeLabel(now: Date, startedAt: Date) -> String {
        let elapsed = max(Int(now.timeIntervalSince(startedAt)), 0)
        if elapsed < 60 {
            return "\(elapsed)s"
        }

        let minutes = elapsed / 60
        if elapsed < 3_600 {
            return "\(minutes)m"
        }

        let hours = minutes / 60
        return "\(hours)h \(minutes % 60)m"
    }
}
