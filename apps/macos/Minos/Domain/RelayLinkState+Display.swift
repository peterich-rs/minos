import SwiftUI

extension RelayLinkState {
    /// Localized one-line description of the link status. The Chinese
    /// strings live here (rather than a Localizable.strings table) so they
    /// stay diff-reviewable alongside the state-machine logic; the spec
    /// freezes them in §6.
    func displayLabel(lang: Lang = .zh) -> String {
        switch (self, lang) {
        case (.disconnected, .zh):
            return "未连接后端"
        case (.disconnected, .en):
            return "Backend disconnected"
        case let (.connecting(attempt), .zh):
            return attempt == 0 ? "正在连接后端…" : "正在重连后端 · 第 \(attempt) 次"
        case let (.connecting(attempt), .en):
            return attempt == 0 ? "Connecting…" : "Reconnecting (attempt \(attempt))"
        case (.connected, .zh):
            return "已连接后端"
        case (.connected, .en):
            return "Backend connected"
        }
    }

    /// SF Symbol used in compact menubar surfaces. The dual-axis status
    /// composer (`StatusIcon`) overlays a peer indicator on top of this.
    var statusSymbolName: String {
        switch self {
        case .disconnected: return "bolt.slash"
        case .connecting: return "bolt.circle"
        case .connected: return "bolt.circle.fill"
        }
    }

    var statusTint: Color {
        switch self {
        case .disconnected: return .secondary
        case .connecting: return .orange
        case .connected: return .green
        }
    }
}
