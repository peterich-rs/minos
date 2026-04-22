import AppKit
import Observation
import OSLog

@Observable
final class AppState: @unchecked Sendable {
    var daemon: (any DaemonDriving)?
    var subscription: (any SubscriptionHandle)?
    var connectionState: ConnectionState?
    var currentQr: QrPayload?
    var currentQrGeneratedAt: Date?
    var trustedDevice: TrustedDevice?
    var bootError: MinosError?
    var displayError: MinosError?
    var isQrSheetPresented = false

    @ObservationIgnored
    private let logger = Logger(subsystem: "ai.minos.macos", category: "appState")

    @ObservationIgnored
    private var displayErrorTask: Task<Void, Never>?

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
        displayError = nil
        bootError = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isQrSheetPresented = false
        trustedDevice = nil
        connectionState = nil
        subscription?.cancel()
        subscription = nil
        daemon = nil
    }

    @MainActor
    func finishBoot(
        daemon: any DaemonDriving,
        subscription: any SubscriptionHandle,
        connectionState: ConnectionState,
        trustedDevice: TrustedDevice?
    ) {
        displayErrorTask?.cancel()
        displayError = nil
        bootError = nil
        self.daemon = daemon
        self.subscription = subscription
        self.connectionState = connectionState
        self.trustedDevice = trustedDevice
    }

    @MainActor
    func failBoot(with error: MinosError) {
        logger.error("Boot failed: \(error.technicalDetails, privacy: .public)")
        subscription?.cancel()
        subscription = nil
        daemon = nil
        connectionState = nil
        trustedDevice = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isQrSheetPresented = false
        bootError = error
    }

    @MainActor
    func applyConnectionState(_ state: ConnectionState) {
        connectionState = state
    }

    @MainActor
    func showQr() async {
        await loadQr(presentingSheet: true)
    }

    @MainActor
    func regenerateQr() async {
        await loadQr(presentingSheet: true)
    }

    @MainActor
    func dismissQrSheet() {
        isQrSheetPresented = false
    }

    @MainActor
    func revealTodayLog() {
        do {
            try DiagnosticsReveal.revealTodayLog()
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
            isQrSheetPresented = false
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            logger.error("Unexpected forget error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func shutdown() async {
        displayErrorTask?.cancel()

        let currentDaemon = daemon
        let currentSubscription = subscription

        daemon = nil
        subscription = nil
        currentQr = nil
        currentQrGeneratedAt = nil
        isQrSheetPresented = false

        do {
            try await currentDaemon?.stop()
        } catch let error as MinosError {
            logger.error("Shutdown stop failed: \(error.technicalDetails, privacy: .public)")
        } catch {
            logger.error("Unexpected shutdown error: \(String(describing: error), privacy: .public)")
        }

        currentSubscription?.cancel()
        terminator()
    }

    private static let qrLifetimeSeconds: TimeInterval = 300

    @MainActor
    private func loadQr(presentingSheet: Bool) async {
        guard canShowQr, let daemon else {
            return
        }

        do {
            let pairingPayload = try daemon.pairingQr()
            currentQr = pairingPayload
            currentQrGeneratedAt = Date()
            isQrSheetPresented = presentingSheet
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
