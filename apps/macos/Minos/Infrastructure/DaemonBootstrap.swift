import Foundation
import OSLog

/// Production daemon bootstrap. Resolves CF Service Token credentials in
/// precedence order (env vars override Keychain), spawns the daemon, and
/// wires the dual-axis observers into AppState.
///
/// If no credentials are found anywhere we surface the onboarding sheet
/// rather than hard-failing — first-run users haven't configured anything
/// yet and the menubar should walk them through it.
///
/// Plan 05 Phase I.6.
enum DaemonBootstrap {
    private static let logger = Logger(subsystem: "ai.minos.macos", category: "bootstrap")

    /// Default startDaemon factory used in production. Reads
    /// LocalState off disk and the device-secret out of Keychain;
    /// Swift only supplies what's not on the filesystem (CF token +
    /// display name) before handing off to the Rust ctor.
    static let defaultStartDaemon: @Sendable (RelayConfig, String) async throws
        -> any DaemonDriving = { config, macName in
        let localStatePath = AppDirectories.localStatePath()
        let (selfDeviceId, peer) = try LocalStateLoader.loadOrInit(at: localStatePath)
        let secret = KeychainDeviceSecret.read()
        return try await DaemonHandle.start(
            config: config,
            selfDeviceId: selfDeviceId,
            peer: peer,
            secret: secret,
            macName: macName
        )
    }

    /// Boot or reboot the daemon. Idempotent — callers from
    /// SettingsSheet's "save & restart" path invoke this after stopping
    /// the previous daemon, and it picks up the freshly written
    /// Keychain creds.
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

        // Env vars override Keychain so dev workflows (`CF_ACCESS_*=…
        // open Minos.app`) work without touching user creds.
        let creds = readEnvCreds() ?? KeychainRelayConfig.read()
        guard let creds else {
            logger.info("No CF credentials present; surfacing onboarding")
            await MainActor.run {
                appState.phase = .awaitingConfig
                appState.onboardingVisible = true
            }
            return
        }

        let config = RelayConfig(
            cfClientId: creds.clientId,
            cfClientSecret: creds.clientSecret
        )

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

    private static func readEnvCreds() -> KeychainRelayConfig.Creds? {
        let env = ProcessInfo.processInfo.environment
        guard
            let id = env["CF_ACCESS_CLIENT_ID"], !id.isEmpty,
            let secret = env["CF_ACCESS_CLIENT_SECRET"], !secret.isEmpty
        else {
            return nil
        }
        return KeychainRelayConfig.Creds(clientId: id, clientSecret: secret)
    }

    private static func hostName() -> String {
        Host.current().localizedName ?? ProcessInfo.processInfo.hostName
    }
}

// ── Local-state JSON loader ──
//
// Bundled into this file because it's only consumed from the bootstrap
// path. The Rust side persists `local-state.json` (DeviceId + optional
// peer record) under `~/Library/Application Support/Minos/`; first run
// creates a fresh DeviceId and writes the file. Swift mirrors that
// behavior so the daemon doesn't have to be running before we know our
// own DeviceId.

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
/// (`crates/minos-daemon/src/local_state.rs`). Snake-case keys match
/// the Rust serde derive verbatim, including the `paired_at` ISO-8601
/// timestamp inside the optional `peer` block.
struct LocalStateJSON: Codable {
    let selfDeviceId: DeviceId
    let peer: PeerRecordJSON?

    enum CodingKeys: String, CodingKey {
        case selfDeviceId = "self_device_id"
        case peer
    }

    func toRecord() -> PeerRecord? {
        peer.map { json in
            PeerRecord(deviceId: json.deviceId, name: json.name, pairedAt: json.pairedAt)
        }
    }
}

/// JSON-side mirror of `PeerRecord`. The UniFFI-generated `PeerRecord`
/// struct is not `Codable` (no auto-derive across the FFI boundary), so
/// we shadow the three live fields and convert at the boundary.
struct PeerRecordJSON: Codable {
    let deviceId: DeviceId
    let name: String
    let pairedAt: Date

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case name
        case pairedAt = "paired_at"
    }
}

enum LocalStateLoader {
    /// Mirror of the Rust `LocalState::load_or_init` semantics. If the
    /// file is missing, mint a fresh DeviceId and persist it; if it's
    /// present but corrupt, surface as a Swift-side throw the bootstrap
    /// catches and converts into a `bootError`.
    static func loadOrInit(at path: URL) throws -> (selfDeviceId: DeviceId, peer: PeerRecord?) {
        let manager = FileManager.default
        if !manager.fileExists(atPath: path.path) {
            let initial = LocalStateJSON(selfDeviceId: UUID().uuidString.lowercased(), peer: nil)
            try save(initial, to: path)
            return (initial.selfDeviceId, nil)
        }
        let data = try Data(contentsOf: path)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let json = try decoder.decode(LocalStateJSON.self, from: data)
        return (json.selfDeviceId, json.toRecord())
    }

    private static func save(_ state: LocalStateJSON, to path: URL) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        try FileManager.default.createDirectory(
            at: path.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try encoder.encode(state).write(to: path, options: .atomic)
    }
}

// ── Keychain device-secret reader ──
//
// Symmetric to KeychainRelayConfig but for the "device-secret" account.
// Only `read` is needed at bootstrap; the daemon owns writes (after a
// successful Pair the relay's response carries the secret and the Rust
// side stores it via `KeychainTrustedDeviceStore::write`).

enum KeychainDeviceSecret {
    static func read() -> DeviceSecret? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: KeychainRelayConfig.service,
            kSecAttrAccount as String: "device-secret",
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
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
