import AppKit
import Foundation
import OSLog

private let actionsLogger = Logger(subsystem: "ai.minos.macos", category: "appState.actions")

/// User-driven actions on AppState (QR mint/refresh, peer forget,
/// shutdown, log reveal). Lives in its own file so the core AppState
/// type body stays under the swiftlint type-body-length cap.
extension AppState {
    static let qrLifetimeSeconds: TimeInterval = 300

    var currentQrExpiresAt: Date? {
        currentQrGeneratedAt?.addingTimeInterval(Self.qrLifetimeSeconds)
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
            actionsLogger.error("Unexpected reveal error: \(String(describing: error), privacy: .public)")
        }
    }

    @MainActor
    func forgetPeer() async {
        guard canForgetPeer, let daemon, let trustedDevice else { return }
        guard forgetConfirmation(trustedDevice) else { return }

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
            actionsLogger.error("Unexpected forget error: \(String(describing: error), privacy: .public)")
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
            actionsLogger.error("Shutdown stop failed: \(error.technicalDetails, privacy: .public)")
        } catch {
            actionsLogger.error("Unexpected shutdown error: \(String(describing: error), privacy: .public)")
        }

        terminator()
    }

    @MainActor
    func presentTransientError(_ error: MinosError) {
        actionsLogger.error("Presenting transient error: \(error.technicalDetails, privacy: .public)")
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

    @MainActor
    private func loadQr(showing: Bool) async {
        guard canShowQr, let daemon else { return }

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
            actionsLogger.error("Unexpected QR error: \(String(describing: error), privacy: .public)")
        }
    }
}
