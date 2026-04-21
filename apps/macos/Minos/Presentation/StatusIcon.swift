import SwiftUI

struct StatusIcon: View {
    let connectionState: ConnectionState?
    let hasBootError: Bool

    var body: some View {
        Image(systemName: symbolName)
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(tint)
            .imageScale(.large)
            .accessibilityLabel("Minos 状态")
    }

    private var symbolName: String {
        if hasBootError {
            return "bolt.circle.trianglebadge.exclamationmark"
        }
        return connectionState?.statusSymbolName ?? "bolt.circle"
    }

    private var tint: Color {
        if hasBootError {
            return .red
        }
        return connectionState?.statusTint ?? .secondary
    }
}
