import SwiftUI

extension PeerState {
    /// Localized one-line description of the peer-pairing state.
    func displayLabel(lang: Lang = .zh) -> String {
        switch (self, lang) {
        case (.unpaired, .zh):
            return "未配对"
        case (.unpaired, .en):
            return "Unpaired"
        case (.pairing, .zh):
            return "等待扫码"
        case (.pairing, .en):
            return "Waiting for scan"
        case let (.paired(_, name, online), .zh):
            return online ? "手机在线 · \(name)" : "手机离线 · \(name)"
        case let (.paired(_, name, online), .en):
            return online ? "iPhone online · \(name)" : "iPhone offline · \(name)"
        }
    }

    /// Convenience peer-name accessor used by `forget_peer` confirmation
    /// dialogs and menu titles.
    var peerName: String? {
        if case let .paired(_, name, _) = self { return name }
        return nil
    }

    /// True only when the peer is known and the relay last reported it as
    /// online. Used by `AppState.canForgetPeer` and friends.
    var isOnline: Bool {
        if case let .paired(_, _, online) = self { return online }
        return false
    }
}
