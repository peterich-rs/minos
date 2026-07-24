import Foundation

/// Production daemon bootstrap. Resolves the runtime backend URL from the
/// bundled Info.plist, spawns the daemon, and wires the dual-axis observers
/// into AppState. The Mac's own `selfDeviceId` is persisted in the shared
/// Rust daemon state path. The daemon's long-lived `deviceSecret` now lives
/// in the Rust-managed secrets store, so Swift no longer needs to load
/// or migrate it at bootstrap.
/// The peer relationship itself lives only on the backend; the daemon
/// repopulates its in-memory peer mirror after each successful WebSocket
/// connect.
///
/// Plan 05 Phase I.6.
enum DaemonBootstrap {
    private static let backendURLKey = "MINOS_BACKEND_URL"

    /// Default startDaemon factory used in production. Reads `selfDeviceId`
    /// off `local-state.json` (minted on first launch). The daemon loads
    /// any persisted `deviceSecret` internally via its own durable store,
    /// so app bootstrap only needs the stable host device id here. The
    /// peer record is no longer persisted — the daemon queries the backend
    /// for it after the WS link comes up.
    static let defaultStartDaemon: @Sendable (RelayConfig, String) async throws
        -> any DaemonDriving = { config, macName in
        let localStatePath = AppDirectories.localStatePath()
        let selfDeviceId = try LocalStateLoader.loadOrInit(
            at: localStatePath,
            legacyPath: AppDirectories.legacyLocalStatePath()
        )
        AppLog.info(
            "bootstrap",
            "Local state ready; path=\(localStatePath.path); selfDeviceId=\(selfDeviceId)"
        )
        return try await DaemonHandle.start(
            config: config,
            selfDeviceId: selfDeviceId,
            peer: nil,
            secret: nil,
            macName: macName
        )
    }

    /// Boot or reboot the daemon. Idempotent — callers can invoke this after
    /// stopping the previous daemon, and it picks up the current process env.
    ///
    /// `startDaemon` is injected so XCTests can substitute MockDaemon
    /// and exercise the bootstrap state ladder without touching the
    /// real Rust runtime.
    static func bootstrap(
        _ appState: AppState,
        clearExistingLogs: Bool = false,
        startDaemon: @escaping @Sendable (RelayConfig, String) async throws -> any DaemonDriving = defaultStartDaemon
    ) async {
        await appState.beginBoot()
        let logCleanup = clearExistingLogs ? StartupLogCleaner.clearExistingLogs() : nil
        try? initLogging()
        logStartupCleanup(logCleanup)
        let macName = hostName()
        AppLog.info("bootstrap", "Bootstrapping daemon for \(macName)")

        let config: RelayConfig
        do {
            config = try relayConfig()
            AppLog.info("bootstrap", "Relay config resolved; \(relayConfigLog(config))")
        } catch let error as MinosError {
            await appState.failBoot(with: error)
            return
        } catch {
            await appState.failBoot(with: .BackendInternal(message: error.localizedDescription))
            return
        }

        await runStart(appState: appState, config: config, macName: macName, startDaemon: startDaemon)
    }

    private static func logStartupCleanup(_ result: StartupLogCleanupResult?) {
        guard let result else { return }

        let summary = "dir=\(result.logDirectory.path); deleted=\(result.deletedCount); skipped=\(result.skippedCount)"
        if result.failures.isEmpty {
            AppLog.info("bootstrap", "Startup log cleanup complete; \(summary)")
        } else {
            AppLog.warn(
                "bootstrap",
                "Startup log cleanup incomplete; \(summary); failures=\(result.failures.joined(separator: " | "))"
            )
        }
    }

    /// Inner half of `bootstrap`: spawn the daemon, wire observers, and
    /// commit / fail-out. Split off so the outer function clears the
    /// swiftlint function-body-length budget.
    private static func runStart(
        appState: AppState,
        config: RelayConfig,
        macName: String,
        startDaemon: @Sendable (RelayConfig, String) async throws -> any DaemonDriving
    ) async {
        var inFlight = InFlight()

        do {
            let daemon = try await startDaemon(config, macName)
            inFlight.daemon = daemon

            let subs = wireObservers(daemon: daemon, appState: appState)
            inFlight.relayLinkSubscription = subs.relayLink
            inFlight.peerSubscription = subs.peer
            inFlight.agentSubscription = subs.agent

            let snapshot = try await snapshot(of: daemon)
            await appState.finishBoot(
                with: snapshot,
                daemon: daemon,
                relayLinkSubscription: subs.relayLink,
                peerSubscription: subs.peer,
                agentSubscription: subs.agent
            )
            AppLog.info("bootstrap", "Boot complete; phase=running")
        } catch let error as MinosError {
            await failBoot(appState: appState, error: error, inFlight: inFlight)
        } catch {
            let wrapped = MinosError.RpcCallFailed(
                method: "swift.bootstrap",
                message: String(describing: error)
            )
            await failBoot(appState: appState, error: wrapped, inFlight: inFlight)
        }
    }

    private struct WiredSubscriptions {
        let relayLink: any SubscriptionHandle
        let peer: any SubscriptionHandle
        let agent: any SubscriptionHandle
    }

    private static func wireObservers(
        daemon: any DaemonDriving,
        appState: AppState
    ) -> WiredSubscriptions {
        let relayObserver = RelayLinkObserver { state in
            Task { @MainActor in appState.applyRelayLink(state) }
        }
        let peerObserver = PeerObserver { state in
            Task { @MainActor in await appState.applyPeer(state) }
        }
        let agentObserver = AgentStateObserverAdapter { state in
            Task { @MainActor in appState.applyAgentState(state) }
        }
        return WiredSubscriptions(
            relayLink: daemon.subscribeRelayLink(relayObserver),
            peer: daemon.subscribePeer(peerObserver),
            agent: daemon.subscribeAgentState(agentObserver)
        )
    }

    private static func snapshot(of daemon: any DaemonDriving) async throws -> AppState.BootSnapshot {
        let relayLink = daemon.currentRelayLink()
        let peer = daemon.currentPeer()
        let agentState = daemon.currentAgentState()
        let agentThread = try await daemon.currentAgentSession()
        let trustedDevice = try await daemon.currentTrustedDevice()
        let peers = try await daemon.currentPeers()
        return AppState.BootSnapshot(
            relayLink: relayLink,
            peer: peer,
            trustedDevice: trustedDevice,
            peers: peers,
            agentState: agentState,
            agentThread: agentThread
        )
    }

    /// Bag of in-flight references the bootstrap acquires before the
    /// daemon hands off to the AppState. Bundled into one parameter so
    /// `failBoot` clears the swiftlint param-count cap.
    private struct InFlight {
        var daemon: (any DaemonDriving)?
        var relayLinkSubscription: (any SubscriptionHandle)?
        var peerSubscription: (any SubscriptionHandle)?
        var agentSubscription: (any SubscriptionHandle)?
    }

    private static func failBoot(
        appState: AppState,
        error: MinosError,
        inFlight: InFlight
    ) async {
        inFlight.relayLinkSubscription?.cancel()
        inFlight.peerSubscription?.cancel()
        inFlight.agentSubscription?.cancel()
        try? await inFlight.daemon?.stop()
        await appState.failBoot(with: error)
    }

    static func relayConfig(
        infoDictionary: [String: Any]? = Bundle.main.infoDictionary,
        env: [String: String] = ProcessInfo.processInfo.environment
    ) throws -> RelayConfig {
        let infoDictionary = infoDictionary ?? [:]
        let backendUrl = infoString(infoDictionary[backendURLKey]) ?? blankToNil(env[backendURLKey]) ?? ""

        return RelayConfig(backendUrl: backendUrl)
    }

    private static func relayConfigLog(_ config: RelayConfig) -> String {
        let trimmed = config.backendUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return "source=baked-rust-default"
        }
        return "source=runtime; backendUrl=\(trimmed)"
    }

    private static func infoString(_ value: Any?) -> String? {
        blankToNil(value as? String)
    }

    private static func blankToNil(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty
        else {
            return nil
        }
        return trimmed
    }

    private static func hostName() -> String {
        Host.current().localizedName ?? ProcessInfo.processInfo.hostName
    }
}

// ── Local-state JSON loader ──
//
// Persists just the Mac's own `selfDeviceId` so it survives relaunch — the
// peer relationship itself comes from the backend after each connect, and
// is never written to disk. Older `local-state.json` files (with a `peer`
// block) deserialize cleanly because serde/Codable both ignore unknown
// keys by default.

enum AppDirectories {
    static func localStatePath(
        env: [String: String] = ProcessInfo.processInfo.environment
    ) -> URL {
        minosHome(env: env)
            .appendingPathComponent("state", isDirectory: true)
            .appendingPathComponent("local-state.json")
    }

    static func logsDirectory(
        env: [String: String] = ProcessInfo.processInfo.environment
    ) -> URL {
        minosHome(env: env)
            .appendingPathComponent("logs", isDirectory: true)
    }

    private static func minosHome(env: [String: String]) -> URL {
        if let raw = env["MINOS_HOME"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !raw.isEmpty {
            return URL(fileURLWithPath: raw, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".minos", isDirectory: true)
    }

    static func legacyLocalStatePath() -> URL {
        let support = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? URL(fileURLWithPath: NSHomeDirectory())
        return support.appendingPathComponent("Minos/local-state.json")
    }
}

struct StartupLogCleanupResult {
    let logDirectory: URL
    let deletedCount: Int
    let skippedCount: Int
    let failures: [String]
}

enum StartupLogCleaner {
    static func clearExistingLogs(
        env: [String: String] = ProcessInfo.processInfo.environment,
        fileManager: FileManager = .default
    ) -> StartupLogCleanupResult {
        let logDirectory = AppDirectories.logsDirectory(env: env)
        var deletedCount = 0
        var skippedCount = 0
        var failures: [String] = []

        do {
            try fileManager.createDirectory(at: logDirectory, withIntermediateDirectories: true)
            let entries = try fileManager.contentsOfDirectory(
                at: logDirectory,
                includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey]
            )

            for entry in entries {
                if shouldSkip(entry) {
                    skippedCount += 1
                    continue
                }
                do {
                    try fileManager.removeItem(at: entry)
                    deletedCount += 1
                } catch {
                    failures.append("\(entry.lastPathComponent): \(error.localizedDescription)")
                }
            }
        } catch {
            failures.append("\(logDirectory.path): \(error.localizedDescription)")
        }

        return StartupLogCleanupResult(
            logDirectory: logDirectory,
            deletedCount: deletedCount,
            skippedCount: skippedCount,
            failures: failures
        )
    }

    private static func shouldSkip(_ entry: URL) -> Bool {
        guard let values = try? entry.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey]) else {
            return false
        }
        return values.isDirectory == true && values.isSymbolicLink != true
    }
}

/// JSON shape mirroring the Rust `LocalState` struct
/// (`crates/minos-daemon/src/local_state.rs`). After the peer-record move
/// to the backend this carries only `selfDeviceId`.
struct LocalStateJSON: Codable {
    let selfDeviceId: DeviceId

    enum CodingKeys: String, CodingKey {
        case selfDeviceId = "self_device_id"
    }
}

enum LocalStateLoader {
    /// Mirror of the Rust `LocalState::load_or_init` semantics. If the
    /// file is missing, mint a fresh DeviceId and persist it; if it's
    /// present but corrupt, surface as a Swift-side throw the bootstrap
    /// catches and converts into a `bootError`.
    static func loadOrInit(at path: URL, legacyPath: URL? = nil) throws -> DeviceId {
        let manager = FileManager.default
        if !manager.fileExists(atPath: path.path),
           let legacyPath,
           manager.fileExists(atPath: legacyPath.path) {
            let migrated = try load(from: legacyPath)
            try save(migrated, to: path)
            AppLog.info(
                "bootstrap",
                "Migrated local state; from=\(legacyPath.path); to=\(path.path)"
            )
            return migrated.selfDeviceId
        }
        if !manager.fileExists(atPath: path.path) {
            let initial = LocalStateJSON(selfDeviceId: UUID().uuidString.lowercased())
            try save(initial, to: path)
            return initial.selfDeviceId
        }
        return try load(from: path).selfDeviceId
    }

    private static func load(from path: URL) throws -> LocalStateJSON {
        let data: Data
        do {
            data = try Data(contentsOf: path)
        } catch {
            throw MinosError.StoreIo(path: path.path, message: error.localizedDescription)
        }
        return try decodePersistedState(data, from: path.path)
    }

    static func decodePersistedState(_ data: Data, from path: String) throws -> LocalStateJSON {
        do {
            return try JSONDecoder().decode(LocalStateJSON.self, from: data)
        } catch {
            throw MinosError.StoreCorrupt(path: path, message: String(describing: error))
        }
    }

    private static func save(_ state: LocalStateJSON, to path: URL) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(
            at: path.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try encoder.encode(state).write(to: path, options: .atomic)
    }
}
