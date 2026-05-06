import AppKit
import Observation

/// Top-level lifecycle phase the menubar UI ladders against. Distinct
/// from the per-axis state (`relayLink`, `peer`) — `phase` answers "are
/// we booting / running / broken?", the axes answer "given we're running,
/// what's the status?".
///
/// Spec §6 freezes the three values and their UI ladder; bootError is
/// a sub-state of `bootFailed` but kept on its own field so stale errors
/// can be cleared without forcing a phase transition.
enum Phase: Sendable {
    case booting
    case running
    case bootFailed
}

@Observable
final class AppState: @unchecked Sendable {
    /// Snapshot of the four observable values DaemonBootstrap reads off
    /// a freshly started daemon. Bundling them into one type keeps
    /// `finishBoot` under the swiftlint parameter-count cap.
    struct BootSnapshot {
        let relayLink: RelayLinkState
        let peer: PeerState
        let trustedDevice: PeerRecord?
        let peers: [HostPeerSummary]
        let agentState: ThreadState
    }

    // ── Daemon + subscriptions ──
    var daemon: (any DaemonDriving)?
    var relayLinkSubscription: (any SubscriptionHandle)?
    var peerSubscription: (any SubscriptionHandle)?
    var agentSubscription: (any SubscriptionHandle)?

    // ── Lifecycle ──
    var phase: Phase = .booting

    // ── Dual-axis state ──
    var relayLink: RelayLinkState = .disconnected
    var peer: PeerState = .unpaired
    var trustedDevice: PeerRecord?
    var peers: [HostPeerSummary] = []

    // ── Pairing UX ──
    var currentQr: RelayQrPayload?
    var currentQrGeneratedAt: Date?
    var isShowingQr: Bool = false

    // ── Agent runtime ──
    var agentState: ThreadState = .idle
    var currentSession: StartAgentResponse?
    var agentError: MinosError?

    // ── Errors ──
    var bootError: MinosError?
    var displayError: MinosError?

    // Internal access (not private) so AppState+Agent.swift can reach
    // them — the agent error is parented on the same task lifecycle.
    @ObservationIgnored
    var displayErrorTask: Task<Void, Never>?

    @ObservationIgnored
    var agentErrorTask: Task<Void, Never>?

    @ObservationIgnored
    let forgetConfirmation: @MainActor @Sendable (PeerRecord) -> Bool

    @ObservationIgnored
    let terminator: @MainActor @Sendable () -> Void

    init(
        forgetConfirmation: (@MainActor @Sendable (PeerRecord) -> Bool)? = nil,
        terminator: (@MainActor @Sendable () -> Void)? = nil
    ) {
        self.forgetConfirmation = forgetConfirmation ?? { peer in
            AppState.defaultForgetConfirmation(peer)
        }
        self.terminator = terminator ?? { NSApp.terminate(nil) }
    }

    // ── Computed gates for menu items ──

    /// Show the "显示配对二维码…" item only when:
    /// - the daemon is running (so we have someone to ask),
    /// - and the relay link is up (so the QR token can actually be minted).
    var canShowQr: Bool {
        guard phase == .running, daemon != nil else { return false }
        if case .connected = relayLink { return true }
        return false
    }

    /// Show the "忘记已配对设备" item only when:
    /// - the daemon is running,
    /// - the relay link is up (so the host can issue ForgetPeer),
    /// - and at least one peer row is currently known.
    var canForgetPeer: Bool {
        guard phase == .running, daemon != nil else { return false }
        guard case .connected = relayLink else { return false }
        return !resolvedPeers.isEmpty
    }

    func canForgetPeerDevice(_ peer: HostPeerSummary) -> Bool {
        guard canForgetPeer else { return false }
        return resolvedPeers.contains(peer)
    }

    /// Show a manual reconnect affordance when the relay task has stopped
    /// retrying and the daemon is still otherwise booted.
    var canReconnectBackend: Bool {
        guard phase == .running, daemon != nil else { return false }
        if case .disconnected = relayLink { return true }
        return false
    }

    // ── Phase transitions ──

    @MainActor
    func beginBoot() {
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()
        displayError = nil
        agentError = nil
        bootError = nil
        agentState = .idle
        currentSession = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isShowingQr = false
        trustedDevice = nil
        peers = []
        relayLink = .disconnected
        peer = .unpaired
        phase = .booting
        relayLinkSubscription?.cancel()
        peerSubscription?.cancel()
        agentSubscription?.cancel()
        relayLinkSubscription = nil
        peerSubscription = nil
        agentSubscription = nil
        daemon = nil
    }

    /// Snapshot-based finishBoot used by DaemonBootstrap. Same effect
    /// as the longer overload below — kept distinct so callers can
    /// stay under the swiftlint parameter-count cap.
    @MainActor
    func finishBoot(
        with snapshot: BootSnapshot,
        daemon: any DaemonDriving,
        relayLinkSubscription: any SubscriptionHandle,
        peerSubscription: any SubscriptionHandle,
        agentSubscription: any SubscriptionHandle
    ) {
        finishBoot(
            daemon: daemon,
            relayLinkSubscription: relayLinkSubscription,
            peerSubscription: peerSubscription,
            relayLink: snapshot.relayLink,
            peer: snapshot.peer,
            trustedDevice: snapshot.trustedDevice,
            peers: snapshot.peers,
            agentSubscription: agentSubscription,
            agentState: snapshot.agentState
        )
    }

    @MainActor
    // swiftlint:disable:next function_parameter_count
    func finishBoot(
        daemon: any DaemonDriving,
        relayLinkSubscription: any SubscriptionHandle,
        peerSubscription: any SubscriptionHandle,
        relayLink: RelayLinkState,
        peer: PeerState,
        trustedDevice: PeerRecord?,
        peers: [HostPeerSummary] = [],
        agentSubscription: (any SubscriptionHandle)? = nil,
        agentState: ThreadState = .idle
    ) {
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()
        displayError = nil
        agentError = nil
        bootError = nil
        self.daemon = daemon
        self.relayLinkSubscription = relayLinkSubscription
        self.peerSubscription = peerSubscription
        self.agentSubscription = agentSubscription
        self.relayLink = relayLink
        let resolvedPeers = peers.isEmpty ? Self.synthesizedPeers(from: trustedDevice, peer: peer) : peers
        self.peers = resolvedPeers
        self.peer = resolvedPeers.isEmpty ? peer : Self.aggregatePeerState(from: resolvedPeers)
        self.agentState = agentState
        self.currentSession = nil
        self.trustedDevice = resolvedPeers.first.map(Self.peerRecord) ?? trustedDevice
        self.phase = .running
    }

    @MainActor
    func failBoot(with error: MinosError) {
        AppLog.error("appState", "Boot failed: \(error.technicalDetails)")
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()
        relayLinkSubscription?.cancel()
        peerSubscription?.cancel()
        agentSubscription?.cancel()
        relayLinkSubscription = nil
        peerSubscription = nil
        agentSubscription = nil
        daemon = nil
        relayLink = .disconnected
        peer = .unpaired
        agentState = .idle
        currentSession = nil
        trustedDevice = nil
        peers = []
        currentQr = nil
        currentQrGeneratedAt = nil
        isShowingQr = false
        agentError = nil
        bootError = error
        phase = .bootFailed
    }

    /// Push from the relay-link observer. Pure assignment — gate logic
    /// reads from the property, not the call site.
    @MainActor
    func applyRelayLink(_ state: RelayLinkState) {
        relayLink = state
    }

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

        if peers.isEmpty {
            currentQr = nil
            currentQrGeneratedAt = nil
            isShowingQr = false
        }
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
            currentQr = nil
            currentQrGeneratedAt = nil
            isShowingQr = false
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

    private static func synthesizedPeers(from trustedDevice: PeerRecord?, peer: PeerState) -> [HostPeerSummary] {
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

    // QR / forget / shutdown / reveal / presentTransientError live in
    // AppState+Actions.swift so the core type body stays under the
    // swiftlint type-body-length cap.
}
