import AppKit
import Foundation

/// User-driven actions on AppState (peer forget, reconnect, shutdown,
/// log reveal). Lives in its own file so the core AppState type body
/// stays under the swiftlint type-body-length cap.
extension AppState {
    @MainActor
    func requestTermination() {
        terminator()
    }

    @MainActor
    func revealTodayLog() async {
        do {
            try await DiagnosticsReveal.revealTodayLog()
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            AppLog.error("appState.actions", "Unexpected reveal error: \(String(describing: error))")
        }
    }

    @MainActor
    func forgetPeer() async {
        guard let firstPeer = resolvedPeers.first else { return }
        let confirmationPeer = Self.peerRecord(firstPeer)
        guard forgetConfirmation(confirmationPeer), let daemon else { return }

        do {
            try await daemon.forgetPeer()
            applyForgottenPeerLocally(firstPeer.mobileDeviceId)
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            AppLog.error("appState.actions", "Unexpected forget error: \(String(describing: error))")
        }
    }

    @MainActor
    func forgetPeerDevice(_ peer: HostPeerSummary) async {
        guard canForgetPeerDevice(peer), let daemon else { return }
        guard forgetConfirmation(Self.peerRecord(peer)) else { return }

        do {
            try await daemon.forgetPeerDevice(peer.mobileDeviceId)
            applyForgottenPeerLocally(peer.mobileDeviceId)
        } catch let error as MinosError {
            presentTransientError(error)
        } catch {
            AppLog.error("appState.actions", "Unexpected forget-peer-device error: \(String(describing: error))")
        }
    }

    @MainActor
    func reconnectBackend() async {
        guard canReconnectBackend else { return }

        let currentDaemon = daemon
        let currentRelayLinkSubscription = relayLinkSubscription
        let currentPeerSubscription = peerSubscription
        let currentAgentSubscription = agentSubscription

        currentRelayLinkSubscription?.cancel()
        currentPeerSubscription?.cancel()
        currentAgentSubscription?.cancel()

        do {
            try await currentDaemon?.stop()
        } catch let error as MinosError {
            AppLog.error("appState.actions", "Reconnect stop failed: \(error.technicalDetails)")
        } catch {
            AppLog.error("appState.actions", "Unexpected reconnect stop error: \(String(describing: error))")
        }

        await DaemonBootstrap.bootstrap(self)
    }

    @MainActor
    func shutdown() async {
        await shutdownForTermination()
        terminator()
    }

    @MainActor
    func shutdownForTermination() async {
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
        agentError = nil

        currentRelayLinkSubscription?.cancel()
        currentPeerSubscription?.cancel()
        currentAgentSubscription?.cancel()

        do {
            try await currentDaemon?.stop()
        } catch let error as MinosError {
            AppLog.error("appState.actions", "Shutdown stop failed: \(error.technicalDetails)")
        } catch {
            AppLog.error("appState.actions", "Unexpected shutdown error: \(String(describing: error))")
        }
    }

    @MainActor
    func presentTransientError(_ error: MinosError) {
        AppLog.error("appState.actions", "Presenting transient error: \(error.technicalDetails)")
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
        alert.informativeText = "忘记 \(peer.name) 后需要在 Desktop 上重新 Link this Mac。继续吗？"
        alert.addButton(withTitle: "取消")
        alert.addButton(withTitle: "忘记")
        return alert.runModal() == .alertSecondButtonReturn
    }

    @MainActor
    private func applyForgottenPeerLocally(_ mobileDeviceId: DeviceId) {
        let remainingPeers = resolvedPeers.filter { $0.mobileDeviceId != mobileDeviceId }
        applyPeersSnapshot(remainingPeers)
        peer = Self.aggregatePeerState(from: remainingPeers)
    }
}
