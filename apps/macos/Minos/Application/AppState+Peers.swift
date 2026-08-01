import Foundation

/// Peer-row snapshot helpers for `AppState`. Lives in its own file so the
/// core type body stays under the swiftlint type-body-length cap.
extension AppState {
    /// Push from the peer observer. `PeerState` is now an invalidation
    /// signal; the authoritative device rows come from `currentPeers()`.
    @MainActor
    func applyPeer(_ state: PeerState) async {
        peer = state

        guard case .pairing = state else {
            applyLegacyPeerState(state)
            await refreshPeersSnapshot(fallbackState: state)
            return
        }
    }

    var resolvedPeers: [HostPeerSummary] {
        if !peers.isEmpty {
            return peers
        }
        return Self.synthesizedPeers(from: trustedDevice, peer: peer)
    }

    @MainActor
    func applyPeersSnapshot(_ peers: [HostPeerSummary]) {
        self.peers = peers
        trustedDevice = peers.first.map(Self.peerRecord)

    }

    @MainActor
    private func refreshPeersSnapshot(fallbackState: PeerState) async {
        guard let daemon else {
            applyLegacyPeerState(fallbackState)
            return
        }

        do {
            let peers = try await daemon.currentPeers()
            if !peers.isEmpty || Self.shouldTreatEmptyPeersAsAuthoritative(for: fallbackState) {
                applyPeersSnapshot(peers)
                peer = Self.aggregatePeerState(from: peers)
                return
            }
        } catch let error as MinosError {
            AppLog.error("appState", "currentPeers failed: \(error.technicalDetails)")
        } catch {
            AppLog.error("appState", "Unexpected currentPeers failure: \(String(describing: error))")
        }

        applyLegacyPeerState(fallbackState)
    }

    @MainActor
    private func applyLegacyPeerState(_ state: PeerState) {
        switch state {
        case let .paired(id, name, _):
            if trustedDevice?.deviceId != id {
                trustedDevice = PeerRecord(deviceId: id, name: name, pairedAt: Date())
            }
        case .unpaired:
            trustedDevice = nil
        case .pairing:
            break
        }
    }

    static func aggregatePeerState(from peers: [HostPeerSummary]) -> PeerState {
        guard let primary = peers.first(where: { $0.online }) ?? peers.first else {
            return .unpaired
        }
        return .paired(
            peerId: primary.mobileDeviceId,
            peerName: primary.mobileDeviceName,
            online: primary.online
        )
    }

    static func peerRecord(_ peer: HostPeerSummary) -> PeerRecord {
        PeerRecord(
            deviceId: peer.mobileDeviceId,
            name: peer.mobileDeviceName,
            pairedAt: Date(timeIntervalSince1970: TimeInterval(peer.pairedAtMs) / 1000)
        )
    }

    static func synthesizedPeers(from trustedDevice: PeerRecord?, peer: PeerState) -> [HostPeerSummary] {
        if let trustedDevice {
            let isOnline: Bool
            switch peer {
            case let .paired(peerId, _, online) where peerId == trustedDevice.deviceId:
                isOnline = online
            default:
                isOnline = false
            }
            return [
                HostPeerSummary(
                    mobileDeviceId: trustedDevice.deviceId,
                    mobileDeviceName: trustedDevice.name,
                    accountEmail: "",
                    pairedAtMs: Int64(trustedDevice.pairedAt.timeIntervalSince1970 * 1000),
                    lastActiveAtMs: Int64(trustedDevice.pairedAt.timeIntervalSince1970 * 1000),
                    online: isOnline
                )
            ]
        }

        if case let .paired(peerId, peerName, online) = peer {
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            return [
                HostPeerSummary(
                    mobileDeviceId: peerId,
                    mobileDeviceName: peerName,
                    accountEmail: "",
                    pairedAtMs: nowMs,
                    lastActiveAtMs: nowMs,
                    online: online
                )
            ]
        }

        return []
    }

    private static func shouldTreatEmptyPeersAsAuthoritative(for state: PeerState) -> Bool {
        switch state {
        case .unpaired:
            return true
        case .paired, .pairing:
            return false
        }
    }
}
