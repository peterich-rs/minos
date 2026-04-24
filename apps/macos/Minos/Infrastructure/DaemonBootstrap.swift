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

    /// Boot or rebooot the daemon. Idempotent — callers from
    /// SettingsSheet's "save & restart" path invoke this after stopping
    /// the previous daemon, and it picks up the freshly written
    /// Keychain creds.
    ///
    /// `startDaemon` is injected so XCTests can substitute MockDaemon
    /// and exercise the bootstrap state ladder without touching the
    /// real Rust runtime.
    static func bootstrap(
        _ appState: AppState,
        startDaemon: @escaping @Sendable (RelayConfig, String) async throws -> any DaemonDriving = {
            config,
            macName in
            // The Rust ctor reads LocalState off disk and the device-secret
            // out of Keychain. Swift only supplies what's not on the
            // filesystem: CF token + display name.
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

        var startedDaemon: (any DaemonDriving)?
        var activeRelayLinkSubscription: (any SubscriptionHandle)?
        var activePeerSubscription: (any SubscriptionHandle)?
        var activeAgentSubscription: (any SubscriptionHandle)?

        do {
            let daemon = try await startDaemon(config, macName)
            startedDaemon = daemon

            let relayObserver = RelayLinkObserver { state in
                Task { @MainActor in
                    appState.applyRelayLink(state)
                }
            }
            let relayLinkSubscription = daemon.subscribeRelayLink(relayObserver)
            activeRelayLinkSubscription = relayLinkSubscription

            let peerObserver = PeerObserver { state in
                Task { @MainActor in
                    appState.applyPeer(state)
                }
            }
            let peerSubscription = daemon.subscribePeer(peerObserver)
            activePeerSubscription = peerSubscription

            let agentObserver = AgentStateObserverAdapter { state in
                Task { @MainActor in
                    appState.applyAgentState(state)
                }
            }
            let agentSubscription = daemon.subscribeAgentState(agentObserver)
            activeAgentSubscription = agentSubscription

            let relayLink = daemon.currentRelayLink()
            let peer = daemon.currentPeer()
            let agentState = daemon.currentAgentState()
            let trustedDevice = try await daemon.currentTrustedDevice()

            await appState.finishBoot(
                daemon: daemon,
                relayLinkSubscription: relayLinkSubscription,
                peerSubscription: peerSubscription,
                relayLink: relayLink,
                peer: peer,
                trustedDevice: trustedDevice,
                agentSubscription: agentSubscription,
                agentState: agentState
            )
            logger.info("Boot complete; phase=running")
        } catch let error as MinosError {
            activeRelayLinkSubscription?.cancel()
            activePeerSubscription?.cancel()
            activeAgentSubscription?.cancel()
            try? await startedDaemon?.stop()
            await appState.failBoot(with: error)
        } catch {
            activeRelayLinkSubscription?.cancel()
            activePeerSubscription?.cancel()
            activeAgentSubscription?.cancel()
            try? await startedDaemon?.stop()
            let wrapped = MinosError.RpcCallFailed(
                method: "swift.bootstrap",
                message: String(describing: error)
            )
            await appState.failBoot(with: wrapped)
        }
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
        let fm = FileManager.default
        if !fm.fileExists(atPath: path.path) {
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
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard
            status == errSecSuccess,
            let data = item as? Data,
            let s = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return DeviceSecret(s)
    }
}
