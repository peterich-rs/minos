import SwiftUI

/// Menubar status icon. Renders one SF Symbol whose shape + tint encodes
/// the join of (relay link state, peer pairing state). Boot-error short
/// circuits to the warning glyph so failures are visible at a glance
/// regardless of the underlying axes.
///
/// The matrix (spec §6, plan J.3):
///
///     link        peer            symbol               tint
///     ────        ────            ──────               ────
///     connected   paired+online   bolt.circle.fill     green
///     connected   paired+offline  bolt.circle          accent
///     connected   pairing         qrcode               accent
///     connected   unpaired        bolt.circle          accent
///     connecting  any             bolt.circle          orange
///     disconnected any            bolt.slash           red
///     bootError   any             exclamationmark…     red
///
/// Plan 05 Phase J.3.
struct StatusIcon: View {
    let link: RelayLinkState
    let peer: PeerState
    let hasBootError: Bool

    var body: some View {
        Image(systemName: symbolName)
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(tint)
            .imageScale(.large)
            .accessibilityLabel("Minos 状态")
    }

    private var symbolName: String {
        if hasBootError { return "exclamationmark.triangle.fill" }

        switch link {
        case .connected:
            switch peer {
            case .unpaired: return "bolt.circle"
            case .pairing: return "qrcode"
            case let .paired(_, _, online):
                return online ? "bolt.circle.fill" : "bolt.circle"
            }
        case .connecting:
            return "bolt.circle"
        case .disconnected:
            return "bolt.slash"
        }
    }

    private var tint: Color {
        if hasBootError { return .red }

        switch link {
        case .connected:
            if case let .paired(_, _, online) = peer, online { return .green }
            return .accentColor
        case .connecting:
            return .orange
        case .disconnected:
            return .red
        }
    }
}
