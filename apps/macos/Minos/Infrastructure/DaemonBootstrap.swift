import Foundation
import OSLog

enum DaemonBootstrap {
    private static let logger = Logger(subsystem: "ai.minos.macos", category: "bootstrap")

    static func bootstrap(
        _ appState: AppState,
        startDaemon: @escaping @Sendable (String) async throws -> any DaemonDriving = { macName in
            try await DaemonHandle.startAutobind(macName: macName)
        }
    ) async {
        await appState.beginBoot()
        try? initLogging()

        let macName = hostName()
        logger.info("Bootstrapping daemon for \(macName, privacy: .public)")

        var startedDaemon: (any DaemonDriving)?
        var activeSubscription: (any SubscriptionHandle)?
        var activeAgentSubscription: (any SubscriptionHandle)?

        do {
            let daemon = try await startDaemon(macName)
            startedDaemon = daemon

            let adapter = ObserverAdapter { state in
                Task { @MainActor in
                    appState.applyConnectionState(state)
                }
            }
            let subscription = daemon.subscribeObserver(adapter)
            activeSubscription = subscription

            let agentAdapter = AgentStateObserverAdapter { state in
                Task { @MainActor in
                    appState.applyAgentState(state)
                }
            }
            let agentSubscription = daemon.subscribeAgentState(agentAdapter)
            activeAgentSubscription = agentSubscription

            let state = daemon.currentState()
            let agentState = daemon.currentAgentState()
            let trustedDevice = try daemon.currentTrustedDevice()

            await appState.finishBoot(
                daemon: daemon,
                subscription: subscription,
                connectionState: state,
                trustedDevice: trustedDevice,
                agentSubscription: agentSubscription,
                agentState: agentState
            )
            logger.info("Boot complete on \(daemon.host(), privacy: .public):\(daemon.port())")
        } catch let error as MinosError {
            activeSubscription?.cancel()
            activeAgentSubscription?.cancel()
            try? await startedDaemon?.stop()
            await appState.failBoot(with: error)
        } catch {
            activeSubscription?.cancel()
            activeAgentSubscription?.cancel()
            try? await startedDaemon?.stop()
            let wrapped = MinosError.RpcCallFailed(method: "swift.bootstrap", message: String(describing: error))
            await appState.failBoot(with: wrapped)
        }
    }

    private static func hostName() -> String {
        Host.current().localizedName ?? ProcessInfo.processInfo.hostName
    }
}
