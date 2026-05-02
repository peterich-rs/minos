import AppKit
import Observation
import OSLog

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

    @ObservationIgnored
    let logger = Logger(subsystem: "ai.minos.macos", category: "appState")

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
    /// - the relay link is up (so the QR token can actually be minted),
    /// - and there is no peer already paired (a second peer would be a
    ///   second pairing, not currently supported).
    var canShowQr: Bool {
        guard phase == .running, daemon != nil else { return false }
        if case .connected = relayLink, case .unpaired = peer { return true }
        return false
    }

    /// Show the "忘记已配对设备" item only when:
    /// - the daemon is running,
    /// - the relay link is up (so the host can issue ForgetPeer),
    /// - and a peer is currently paired.
    var canForgetPeer: Bool {
        guard phase == .running, daemon != nil else { return false }
        guard case .connected = relayLink else { return false }
        if case .paired = peer { return true }
        return false
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
        self.peer = peer
        self.agentState = agentState
        self.currentSession = nil
        self.trustedDevice = trustedDevice
        self.phase = .running
    }

    @MainActor
    func failBoot(with error: MinosError) {
        logger.error("Boot failed: \(error.technicalDetails, privacy: .public)")
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

    /// Push from the peer observer. Mirror the trustedDevice cache so
    /// pairing-aware UI doesn't have to wait for `current_trusted_device`
    /// to round-trip the daemon.
    ///
    /// `PeerState::Paired` is ephemeral — it carries `(id, name, online)`
    /// but no `pairedAt`, because the relay re-emits Paired/PeerOnline on
    /// every reconnect. `PeerRecord` is the persisted shape and owns the
    /// real `pairedAt` that was captured at first-pair time. So we only
    /// synthesize a new `PeerRecord` when the deviceId has genuinely
    /// changed (first pair, or pair-after-forget). For the steady-state
    /// reconnect case where we already hold a record for this peer, we
    /// leave `trustedDevice` untouched rather than stamp a fresh `Date()`
    /// over the original timestamp.
    @MainActor
    func applyPeer(_ state: PeerState) {
        peer = state
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

    // QR / forget / shutdown / reveal / presentTransientError live in
    // AppState+Actions.swift so the core type body stays under the
    // swiftlint type-body-length cap.
}
