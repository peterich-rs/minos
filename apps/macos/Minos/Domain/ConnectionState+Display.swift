import SwiftUI

extension ConnectionState {
    func displayLabel(lang: Lang = .zh) -> String {
        switch (self, lang) {
        case (.disconnected, .zh):
            return "未连接"
        case (.disconnected, .en):
            return "Disconnected"
        case (.pairing, .zh):
            return "等待手机扫码"
        case (.pairing, .en):
            return "Waiting For Pairing"
        case (.connected, .zh):
            return "已连接"
        case (.connected, .en):
            return "Connected"
        case let (.reconnecting(attempt), .zh):
            return "正在重连（第\(attempt)次）"
        case let (.reconnecting(attempt), .en):
            return "Reconnecting (attempt \(attempt))"
        }
    }

    var statusSymbolName: String {
        switch self {
        case .disconnected:
            return "bolt.circle"
        case .pairing:
            return "bolt.circle.fill"
        case .connected:
            return "bolt.circle.fill"
        case .reconnecting:
            return "bolt.circle.fill"
        }
    }

    var statusTint: Color {
        switch self {
        case .disconnected:
            return .secondary
        case .pairing:
            return .accentColor
        case .connected:
            return .green
        case .reconnecting:
            return .orange
        }
    }
}
