import AppKit
import Observation
import OSLog

/// Top-level lifecycle phase the menubar UI ladders against. Distinct
/// from the per-axis state (`relayLink`, `peer`) — `phase` answers "are
/// we configured / running / broken?", the axes answer "given we're
/// running, what's the status?".
///
/// Spec §6 freezes the three values and their UI ladder; bootError is
/// a sub-state of `bootFailed` but kept on its own field so stale errors
/// can be cleared without forcing a phase transition.
enum Phase: Sendable {
    case awaitingConfig
    case running
    case bootFailed
}

@Observable
final class AppState: @unchecked Sendable {
    // ── Daemon + subscriptions ──
    var daemon: (any DaemonDriving)?
    var relayLinkSubscription: (any SubscriptionHandle)?
    var peerSubscription: (any SubscriptionHandle)?
    var agentSubscription: (any SubscriptionHandle)?

    // ── Lifecycle ──
    var phase: Phase = .awaitingConfig

    // ── Dual-axis state ──
    var relayLink: RelayLinkState = .disconnected
    var peer: PeerState = .unpaired
    var trustedDevice: PeerRecord?

    // ── Pairing UX ──
    var currentQr: RelayQrPayload?
    var currentQrGeneratedAt: Date?
    var isShowingQr: Bool = false
    var onboardingVisible: Bool = false
    var settingsVisible: Bool = false

    // ── Agent runtime ──
    var agentState: AgentState = .idle
    var currentSession: StartAgentResponse?
    var agentError: MinosError?

    // ── Errors ──
    var bootError: MinosError?
    var displayError: MinosError?

    @ObservationIgnored
    private let logger = Logger(subsystem: "ai.minos.macos", category: "appState")

    @ObservationIgnored
    private var displayErrorTask: Task<Void, Never>?

    @ObservationIgnored
    private var agentErrorTask: Task<Void, Never>?

    @ObservationIgnored
    private let forgetConfirmation: @MainActor @Sendable (PeerRecord) -> Bool

    @ObservationIgnored
    private let terminator: @MainActor @Sendable () -> Void

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
        phase = .awaitingConfig
        relayLinkSubscription?.cancel()
        peerSubscription?.cancel()
        agentSubscription?.cancel()
        relayLinkSubscription = nil
        peerSubscription = nil
        agentSubscription = nil
        daemon = nil
    }

    @MainActor
    func finishBoot(
        daemon: any DaemonDriving,
        relayLinkSubscription: any SubscriptionHandle,
        peerSubscription: any SubscriptionHandle,
        relayLink: RelayLinkState,
        peer: PeerState,
        trustedDevice: PeerRecord?,
        agentSubscription: (any SubscriptionHandle)? = nil,
        agentState: AgentState = .idle
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
    @MainActor
    func applyPeer(_ state: PeerState) {
        peer = state
        switch state {
        case let .paired(id, name, _):
            trustedDevice = PeerRecord(deviceId: id, name: name, pairedAt: Date())
        case .unpaired:
            trustedDevice = nil
            currentQr = nil
            currentQrGeneratedAt = nil
            isShowingQr = false
        case .pairing:
            break
        }
    }

    @MainActor
    func showQr() async {
        await loadQr(showing: true)
    }

    @MainActor
    func regenerateQr() async {
        await loadQr(showing: true)
    }

    @MainActor
    func dismissQr() {
        isShowingQr = false
    }

    @MainActor
    func revealTodayLog() async {
        do {
            try await DiagnosticsReveal.revealTodayLog()
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            logger.error("Unexpected reveal error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func forgetPeer() async {
        guard canForgetPeer, let daemon, let trustedDevice else {
            return
        }
        guard forgetConfirmation(trustedDevice) else {
            return
        }

        do {
            try await daemon.forgetPeer()
            // The daemon's peer observer will push Unpaired shortly; the
            // local clear here is belt-and-suspenders so menus refresh
            // synchronously even if the observer is briefly delayed.
            self.peer = .unpaired
            self.trustedDevice = nil
            currentQr = nil
            currentQrGeneratedAt = nil
            isShowingQr = false
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            logger.error("Unexpected forget error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func shutdown() async {
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()

        let currentDaemon = daemon
        let currentRelayLinkSubscription = relayLinkSubscription
        let currentPeerSubscription = peerSubscription
        let currentAgentSubscription = agentSubscription

        daemon = nil
        relayLinkSubscription = nil
        peerSubscription = nil
        agentSubscription = nil
        agentState = .idle
        currentSession = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isShowingQr = false
        agentError = nil

        currentRelayLinkSubscription?.cancel()
        currentPeerSubscription?.cancel()
        currentAgentSubscription?.cancel()

        do {
            try await currentDaemon?.stop()
        } catch let error as MinosError {
            logger.error("Shutdown stop failed: \(error.technicalDetails, privacy: .public)")
        } catch {
            logger.error("Unexpected shutdown error: \(String(describing: error), privacy: .public)")
        }

        terminator()
    }

    private static let qrLifetimeSeconds: TimeInterval = 300

    @MainActor
    private func loadQr(showing: Bool) async {
        guard canShowQr, let daemon else {
            return
        }

        do {
            let pairingPayload = try await daemon.pairingQr()
            currentQr = pairingPayload
            currentQrGeneratedAt = Date()
            isShowingQr = showing
            displayErrorTask?.cancel()
            displayError = nil
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            logger.error("Unexpected QR error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    private func presentTransientError(_ error: MinosError) {
        logger.error("Presenting transient error: \(error.technicalDetails, privacy: .public)")
        displayErrorTask?.cancel()
        displayError = error
        displayErrorTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            await MainActor.run {
                self?.displayError = nil
            }
        }
    }

    @MainActor
    static func defaultForgetConfirmation(_ peer: PeerRecord) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "忘记已配对设备"
        alert.informativeText = "忘记 \(peer.name) 后需要重新扫码才能再次配对。继续吗？"
        alert.addButton(withTitle: "取消")
        alert.addButton(withTitle: "忘记")
        return alert.runModal() == .alertSecondButtonReturn
    }

    var currentQrExpiresAt: Date? {
        currentQrGeneratedAt?.addingTimeInterval(Self.qrLifetimeSeconds)
    }
}

extension AppState {
    @MainActor
    func applyAgentState(_ state: AgentState) {
        agentState = state

        switch state {
        case .idle, .crashed:
            currentSession = nil
        case .starting, .running, .stopping:
            break
        }
    }

    @MainActor
    func startAgent() async {
        guard phase == .running, let daemon else {
            return
        }

        clearAgentError()
        currentSession = nil

        do {
            currentSession = try await daemon.startAgent(.init(agent: .codex))
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            presentAgentError(error)
        } catch {
            logger.error("Unexpected start-agent error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func sendAgentPing() async {
        guard phase == .running, let daemon, let currentSession else {
            return
        }

        clearAgentError()

        do {
            try await daemon.sendUserMessage(.init(sessionId: currentSession.sessionId, text: "ping"))
        } catch let error as MinosError {
            self.currentSession = nil
            agentState = .idle
            presentAgentError(error)
        } catch {
            logger.error("Unexpected agent ping error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func stopAgent() async {
        guard phase == .running, let daemon else {
            return
        }

        clearAgentError()

        do {
            try await daemon.stopAgent()
            currentSession = nil
        } catch let error as MinosError {
            currentSession = nil
            agentState = .idle
            presentAgentError(error)
        } catch {
            logger.error("Unexpected stop-agent error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func dismissAgentCrash() {
        clearAgentError()
    }
}

private extension AppState {
    @MainActor
    func clearAgentError() {
        agentErrorTask?.cancel()
        agentError = nil
    }

    @MainActor
    func presentAgentError(_ error: MinosError) {
        logger.error("Presenting agent error: \(error.technicalDetails, privacy: .public)")
        agentErrorTask?.cancel()
        agentError = error
        agentErrorTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            await MainActor.run {
                self?.agentError = nil
            }
        }
    }
}
