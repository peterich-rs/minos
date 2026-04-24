import AppKit
import Observation
import OSLog

@Observable
final class AppState: @unchecked Sendable {
    var daemon: (any DaemonDriving)?
    var subscription: (any SubscriptionHandle)?
    var agentSubscription: (any SubscriptionHandle)?
    var connectionState: ConnectionState?
    var agentState: AgentState = .idle
    var currentQr: QrPayload?
    var currentQrGeneratedAt: Date?
    var currentSession: StartAgentResponse?
    var trustedDevice: TrustedDevice?
    var agentError: MinosError?
    var bootError: MinosError?
    var displayError: MinosError?
    var isShowingQr = false

    @ObservationIgnored
    private let logger = Logger(subsystem: "ai.minos.macos", category: "appState")

    @ObservationIgnored
    private var displayErrorTask: Task<Void, Never>?

    @ObservationIgnored
    private var agentErrorTask: Task<Void, Never>?

    @ObservationIgnored
    private let forgetConfirmation: @MainActor @Sendable (TrustedDevice) -> Bool

    @ObservationIgnored
    private let terminator: @MainActor @Sendable () -> Void

    init(
        forgetConfirmation: (@MainActor @Sendable (TrustedDevice) -> Bool)? = nil,
        terminator: (@MainActor @Sendable () -> Void)? = nil
    ) {
        self.forgetConfirmation = forgetConfirmation ?? { trustedDevice in
            AppState.defaultForgetConfirmation(trustedDevice)
        }
        self.terminator = terminator ?? { NSApp.terminate(nil) }
    }

    var canShowQr: Bool {
        bootError == nil && trustedDevice == nil && daemon != nil
    }

    var canForgetDevice: Bool {
        bootError == nil && trustedDevice != nil && daemon != nil
    }

    var endpointDisplay: String? {
        guard bootError == nil, trustedDevice == nil, let daemon else {
            return nil
        }
        return "\(daemon.host()):\(daemon.port())"
    }

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
        connectionState = nil
        subscription?.cancel()
        agentSubscription?.cancel()
        subscription = nil
        agentSubscription = nil
        daemon = nil
    }

    @MainActor
    func finishBoot(
        daemon: any DaemonDriving,
        subscription: any SubscriptionHandle,
        connectionState: ConnectionState,
        trustedDevice: TrustedDevice?,
        agentSubscription: (any SubscriptionHandle)? = nil,
        agentState: AgentState = .idle
    ) {
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()
        displayError = nil
        agentError = nil
        bootError = nil
        self.daemon = daemon
        self.subscription = subscription
        self.agentSubscription = agentSubscription
        self.connectionState = connectionState
        self.agentState = agentState
        currentSession = nil
        self.trustedDevice = trustedDevice
    }

    @MainActor
    func failBoot(with error: MinosError) {
        logger.error("Boot failed: \(error.technicalDetails, privacy: .public)")
        displayErrorTask?.cancel()
        agentErrorTask?.cancel()
        subscription?.cancel()
        agentSubscription?.cancel()
        subscription = nil
        agentSubscription = nil
        daemon = nil
        connectionState = nil
        agentState = .idle
        currentSession = nil
        trustedDevice = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isShowingQr = false
        agentError = nil
        bootError = error
    }

    @MainActor
    func applyConnectionState(_ state: ConnectionState) {
        connectionState = state
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
    func forgetDevice() async {
        guard bootError == nil, let daemon, let trustedDevice else {
            return
        }
        guard forgetConfirmation(trustedDevice) else {
            return
        }

        do {
            try await daemon.forgetDevice(id: trustedDevice.deviceId)
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
        let currentSubscription = subscription
        let currentAgentSubscription = agentSubscription

        daemon = nil
        subscription = nil
        agentSubscription = nil
        agentState = .idle
        currentSession = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isShowingQr = false
        agentError = nil

        currentSubscription?.cancel()
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
            let pairingPayload = try daemon.pairingQr()
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
    static func defaultForgetConfirmation(_ trustedDevice: TrustedDevice) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "忘记已配对设备"
        alert.informativeText = "忘记 \(trustedDevice.name) 后需要重新扫码才能再次配对。继续吗？"
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
        guard bootError == nil, let daemon else {
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
        guard bootError == nil, let daemon, let currentSession else {
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
        guard bootError == nil, let daemon else {
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
