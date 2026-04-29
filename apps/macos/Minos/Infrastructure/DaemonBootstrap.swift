import Foundation
import OSLog
import Security

/// Production daemon bootstrap. Resolves CF Service Token credentials from the
/// bundled Info.plist, spawns the daemon, and wires the dual-axis observers
/// into AppState. The Mac's own `selfDeviceId` is persisted in
/// `local-state.json` and `deviceSecret` in the macOS Keychain — both are
/// device credentials, not pairing facts, so they survive relaunch. The
/// peer relationship itself lives only on the backend; the daemon
/// repopulates its in-memory peer mirror via `GET /v1/me/peer` after each
/// successful WebSocket connect.
///
/// Plan 05 Phase I.6.
enum DaemonBootstrap {
    private static let logger = Logger(subsystem: "ai.minos.macos", category: "bootstrap")
    fileprivate static let keychainService = "ai.minos.macos"
    private static let backendURLKey = "MINOS_BACKEND_URL"
    private static let cfClientIdKey = "CF_ACCESS_CLIENT_ID"
    private static let cfClientSecretKey = "CF_ACCESS_CLIENT_SECRET"

    /// Default startDaemon factory used in production. Reads `selfDeviceId`
    /// off `local-state.json` (minted on first launch) and `deviceSecret`
    /// out of the Keychain. The peer record is no longer persisted — the
    /// daemon queries the backend for it after the WS link comes up.
    static let defaultStartDaemon: @Sendable (RelayConfig, String) async throws
        -> any DaemonDriving = { config, macName in
        let localStatePath = AppDirectories.localStatePath()
        let selfDeviceId = try LocalStateLoader.loadOrInit(at: localStatePath)
        let secret = KeychainDeviceSecret.read()
        return try await DaemonHandle.start(
            config: config,
            selfDeviceId: selfDeviceId,
            peer: nil,
            secret: secret,
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
        startDaemon: @escaping @Sendable (RelayConfig, String) async throws -> any DaemonDriving = defaultStartDaemon
    ) async {
        await appState.beginBoot()
        try? initLogging()
        let macName = hostName()
        logger.info("Bootstrapping daemon for \(macName, privacy: .public)")

        let config: RelayConfig
        do {
            config = try relayConfig()
        } catch let error as MinosError {
            await appState.failBoot(with: error)
            return
        } catch {
            await appState.failBoot(with: .BackendInternal(message: error.localizedDescription))
            return
        }

        await runStart(appState: appState, config: config, macName: macName, startDaemon: startDaemon)
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
            logger.info("Boot complete; phase=running")
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
            Task { @MainActor in appState.applyPeer(state) }
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
        let trustedDevice = try await daemon.currentTrustedDevice()
        return AppState.BootSnapshot(
            relayLink: relayLink,
            peer: peer,
            trustedDevice: trustedDevice,
            agentState: agentState
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

    struct CfAccessCreds {
        let clientId: String
        let clientSecret: String
    }

    static func relayConfig(
        infoDictionary: [String: Any]? = Bundle.main.infoDictionary
    ) throws -> RelayConfig {
        let infoDictionary = infoDictionary ?? [:]
        let creds = try infoCreds(from: infoDictionary)
        let backendUrl = infoString(infoDictionary[backendURLKey]) ?? ""

        return RelayConfig(
            backendUrl: backendUrl,
            cfClientId: creds?.clientId ?? "",
            cfClientSecret: creds?.clientSecret ?? ""
        )
    }

    static func envCreds(from env: [String: String] = ProcessInfo.processInfo.environment) throws -> CfAccessCreds? {
        try creds(
            clientId: blankToNil(env[cfClientIdKey]),
            clientSecret: blankToNil(env[cfClientSecretKey]),
            source: "process environment"
        )
    }

    static func infoCreds(from infoDictionary: [String: Any]) throws -> CfAccessCreds? {
        try creds(
            clientId: infoString(infoDictionary[cfClientIdKey]),
            clientSecret: infoString(infoDictionary[cfClientSecretKey]),
            source: "Info.plist"
        )
    }

    private static func creds(
        clientId: String?,
        clientSecret: String?,
        source: String
    ) throws -> CfAccessCreds? {
        let id = blankToNil(clientId)
        let secret = blankToNil(clientSecret)

        switch (id, secret) {
        case let (.some(id), .some(secret)):
            return CfAccessCreds(clientId: id, clientSecret: secret)
        case (nil, nil):
            return nil
        case (.some, nil):
            throw MinosError.CfAccessMisconfigured(
                reason: "CF_ACCESS_CLIENT_ID is set but CF_ACCESS_CLIENT_SECRET is missing in \(source)"
            )
        case (nil, .some):
            throw MinosError.CfAccessMisconfigured(
                reason: "CF_ACCESS_CLIENT_SECRET is set but CF_ACCESS_CLIENT_ID is missing in \(source)"
            )
        }
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
    static func localStatePath() -> URL {
        let support = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? URL(fileURLWithPath: NSHomeDirectory())
        return support.appendingPathComponent("Minos/local-state.json")
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
    static func loadOrInit(at path: URL) throws -> DeviceId {
        let manager = FileManager.default
        if !manager.fileExists(atPath: path.path) {
            let initial = LocalStateJSON(selfDeviceId: UUID().uuidString.lowercased())
            try save(initial, to: path)
            return initial.selfDeviceId
        }
        let data: Data
        do {
            data = try Data(contentsOf: path)
        } catch {
            throw MinosError.StoreIo(path: path.path, message: error.localizedDescription)
        }

        let json = try decodePersistedState(data, from: path.path)
        return json.selfDeviceId
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

// ── Keychain device-secret reader ──
//
// Only `read` is needed at bootstrap; the daemon owns writes (after a
// successful Pair the relay's response carries the secret and the Rust
// side stores it via `KeychainTrustedDeviceStore::write`). The query skips
// authentication UI so a background menu-bar launch never prompts.

enum KeychainDeviceSecret {
    static func read() -> DeviceSecret? {
        if let secret = readNoUI(useProtectedKeychain: true) {
            return secret
        }
        return readNoUI(useProtectedKeychain: false)
    }

    private static func readNoUI(useProtectedKeychain: Bool) -> DeviceSecret? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: DaemonBootstrap.keychainService,
            kSecAttrAccount as String: "device-secret",
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
            kSecUseAuthenticationUI as String: kSecUseAuthenticationUISkip,
            kSecUseDataProtectionKeychain as String: useProtectedKeychain
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard
            status == errSecSuccess,
            let data = item as? Data,
            let utf8 = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return DeviceSecret(utf8)
    }
}
