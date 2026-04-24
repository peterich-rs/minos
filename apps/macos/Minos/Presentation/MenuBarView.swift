import AppKit
import SwiftUI

/// Menubar popover content. Walks the AppState.phase ladder
/// (awaitingConfig → running → bootFailed) and within `.running`
/// renders the dual-axis status + action items gated by canShowQr /
/// canForgetPeer.
///
/// Plan 05 Phase J.2.
struct MenuBarView: View {
    @Bindable var appState: AppState
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let displayError = appState.displayError, appState.bootError == nil {
                errorBanner(displayError)
            }

            switch appState.phase {
            case .awaitingConfig:
                awaitingConfigContent
            case .bootFailed:
                bootErrorContent(appState.bootError)
            case .running:
                runningContent
            }
        }
        .padding(14)
        .frame(width: 360)
        // Onboarding + Settings are declared as App-level Window scenes,
        // not .sheet() from here — see MinosApp.swift for the why.
    }

    /// Open the onboarding / settings window. LSUIElement apps need an
    /// explicit `NSApp.activate` so the new window becomes key and its
    /// TextFields can receive focus; without it the window opens behind
    /// the current frontmost app.
    private func openAuxiliaryWindow(_ id: String) {
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: id)
    }

    // ── Phase: awaiting configuration ────────────────────────────────

    private var awaitingConfigContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: "bolt.slash")
                    .foregroundStyle(.red)
                    .imageScale(.large)
                Text("Minos · 等待配置")
                    .font(.headline)
            }

            Text("尚未配置 Cloudflare Service Token，请先填入 Relay 设置后重试。")
                .font(.caption)
                .foregroundStyle(.secondary)

            Divider()

            actionButton("Relay 设置…") {
                openAuxiliaryWindow(WindowID.onboarding)
            }
            actionButton("退出 Minos") {
                Task { await appState.shutdown() }
            }
        }
    }

    // ── Phase: running ──────────────────────────────────────────────

    @ViewBuilder
    private var runningContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            header

            Divider()

            if appState.isShowingQr {
                PairingQRView(appState: appState)
            } else {
                runningActions
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                StatusIcon(
                    link: appState.relayLink,
                    peer: appState.peer,
                    hasBootError: false
                )
                Text("Minos")
                    .font(.headline)
            }

            Text(appState.relayLink.displayLabel())
                .font(.subheadline.weight(.medium))

            Text(appState.peer.displayLabel())
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var runningActions: some View {
        VStack(alignment: .leading, spacing: 12) {
            if appState.canShowQr {
                actionButton("显示配对二维码…") {
                    Task { await appState.showQr() }
                }
            }

            if appState.canForgetPeer {
                actionButton("忘记已配对设备", role: .destructive) {
                    Task { await appState.forgetPeer() }
                }
            } else if case .paired = appState.peer {
                // Paired but the link is not connected — surface a
                // disabled affordance so users know the action exists
                // and what they need to do (wait for reconnect) before
                // it becomes available.
                actionButton("忘记已配对设备 (需要后端在线)", action: {})
                    .disabled(true)
            }

            AgentSegmentView(appState: appState)

            actionButton("Relay 设置…") {
                openAuxiliaryWindow(WindowID.settings)
            }
            actionButton("在 Finder 中显示今日日志…") {
                Task { await appState.revealTodayLog() }
            }

            Divider()

            actionButton("退出 Minos") {
                Task { await appState.shutdown() }
            }
        }
    }

    // ── Phase: boot failed ──────────────────────────────────────────

    private func bootErrorContent(_ bootError: MinosError?) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                StatusIcon(
                    link: .disconnected,
                    peer: .unpaired,
                    hasBootError: true
                )
                Text("Minos · 启动失败")
                    .font(.headline)
            }

            if let bootError {
                Text(bootError.userMessage(lang: .zh))
                    .font(.subheadline)

                DisclosureGroup("详情") {
                    Text(bootError.technicalDetails)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 4)
                }
            } else {
                Text("未知启动错误")
                    .font(.subheadline)
                    .foregroundStyle(.red)
            }

            Button("重试") {
                Task { await DaemonBootstrap.bootstrap(appState) }
            }
            .keyboardShortcut(.defaultAction)

            Divider()

            actionButton("Relay 设置…") {
                openAuxiliaryWindow(WindowID.settings)
            }
            actionButton("在 Finder 中显示今日日志…") {
                Task { await appState.revealTodayLog() }
            }
            actionButton("退出 Minos") {
                Task { await appState.shutdown() }
            }
        }
    }

    // ── Shared widgets ──────────────────────────────────────────────

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

    private func actionButton(
        _ title: String,
        role: ButtonRole? = nil,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            Text(title)
                .foregroundStyle(role == .destructive ? Color.red : Color.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
