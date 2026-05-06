import AppKit
import SwiftUI

/// Menubar popover content. Walks the AppState.phase ladder
/// (booting → running → bootFailed) and within `.running` renders the
/// dual-axis status + action items gated by canShowQr / canForgetPeer.
///
/// Plan 05 Phase J.2.
struct MenuBarView: View {
    @Bindable var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let displayError = appState.displayError, appState.bootError == nil {
                errorBanner(displayError)
            }

            switch appState.phase {
            case .booting:
                bootingContent
            case .bootFailed:
                bootErrorContent(appState.bootError)
            case .running:
                runningContent
            }
        }
        .padding(14)
        .frame(width: 360)
    }

    // ── Phase: booting ────────────────────────────────────────────────

    private var bootingContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                ProgressView()
                    .controlSize(.small)
                Text("Minos · 正在启动")
                    .font(.headline)
            }

            Text("正在启动 agent host 并连接后端。Cloudflare Access 凭据仅从环境变量读取。")
                .font(.caption)
                .foregroundStyle(.secondary)

            Divider()

            actionButton("退出 Minos") {
                appState.requestTermination()
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
                    peer: .unpaired,
                    hasBootError: false
                )
                .imageScale(.large)
                Text("Minos")
                    .font(.headline)
            }

            Text("Host 与 Server")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            Text(appState.relayLink.displayLabel())
                .font(.subheadline.weight(.medium))

            Text(
                appState.resolvedPeers.isEmpty
                    ? "暂无接入设备"
                    : "\(appState.resolvedPeers.count) 台设备已接入"
            )
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var runningActions: some View {
        VStack(alignment: .leading, spacing: 12) {
            pairedDevicesSection

            if appState.canShowQr {
                actionButton("显示配对二维码…") {
                    Task { await appState.showQr() }
                }
            }

            if appState.canReconnectBackend {
                actionButton("重新连接后端") {
                    Task { await appState.reconnectBackend() }
                }
            }

            AgentSegmentView(appState: appState)

            actionButton("在 Finder 中显示今日日志…") {
                Task { await appState.revealTodayLog() }
            }

            Divider()

            actionButton("退出 Minos") {
                appState.requestTermination()
            }
        }
    }

    private var pairedDevicesSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("接入设备")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            if appState.resolvedPeers.isEmpty {
                Text("当前还没有 mobile 设备接入这个 host。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: 10)
                            .fill(Color(nsColor: .controlBackgroundColor))
                    )
            } else {
                ForEach(appState.resolvedPeers, id: \.mobileDeviceId) { peer in
                    pairedDeviceRow(peer)
                }
            }
        }
    }

    private func pairedDeviceRow(_ peer: HostPeerSummary) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(peer.online ? Color.green : Color.secondary.opacity(0.45))
                .frame(width: 8, height: 8)
                .padding(.top, 5)

            VStack(alignment: .leading, spacing: 4) {
                Text(peerTitle(peer))
                    .font(.subheadline.weight(.medium))
                    .fixedSize(horizontal: false, vertical: true)

                Text(peerSubtitle(peer))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 8)

            Button(role: .destructive) {
                Task { await appState.forgetPeerDevice(peer) }
            } label: {
                Image(systemName: "trash")
                    .foregroundStyle(appState.canForgetPeerDevice(peer) ? Color.red : Color.secondary)
            }
            .buttonStyle(.plain)
            .disabled(!appState.canForgetPeerDevice(peer))
            .help(appState.canForgetPeerDevice(peer) ? "移除此设备" : "需要后端在线")
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Color(nsColor: .controlBackgroundColor))
        )
    }

    private func peerTitle(_ peer: HostPeerSummary) -> String {
        if peer.accountEmail.isEmpty {
            return peer.mobileDeviceName
        }
        if peer.mobileDeviceName.isEmpty {
            return peer.accountEmail
        }
        return "\(peer.accountEmail) · \(peer.mobileDeviceName)"
    }

    private func peerSubtitle(_ peer: HostPeerSummary) -> String {
        let status = peer.online ? "在线" : "离线"
        return "\(status) · \(lastActiveText(peer))"
    }

    private func lastActiveText(_ peer: HostPeerSummary) -> String {
        guard peer.lastActiveAtMs > 0 else {
            return "最后活跃未知"
        }
        let date = Date(timeIntervalSince1970: TimeInterval(peer.lastActiveAtMs) / 1000)
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = Locale(identifier: "zh_CN")
        return "最后活跃 \(formatter.localizedString(for: date, relativeTo: Date()))"
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

            actionButton("在 Finder 中显示今日日志…") {
                Task { await appState.revealTodayLog() }
            }
            actionButton("退出 Minos") {
                appState.requestTermination()
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
