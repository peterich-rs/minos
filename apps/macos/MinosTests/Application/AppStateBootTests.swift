import XCTest

@testable import Minos

/// Boot-side coverage of `AppState`: phase transitions and the
/// dual-axis observer push pipeline. Pairing- and gate-specific
/// scenarios live in `AppStateActionTests`.
final class AppStateBootTests: XCTestCase {
    @MainActor
    func testBeginBootResetsRuntimeStateAndCancelsExistingSubscriptions() {
        let appState = AppState()
        let daemon = MockDaemon()
        let relayLinkSub = MockSubscription()
        let peerSub = MockSubscription()

        appState.daemon = daemon
        appState.relayLinkSubscription = relayLinkSub
        appState.peerSubscription = peerSub
        appState.relayLink = .connected
        appState.peer = .paired(
            peerId: "00000000-0000-0000-0000-000000000001",
            peerName: "Existing iPhone",
            online: true
        )
        appState.currentQr = MockDaemon.makeQrPayload(macDisplayName: "Old Mac")
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 123)
        appState.trustedDevice = MockDaemon.makeTrustedDevice(name: "Existing Device")
        appState.bootError = .StoreIo(path: "/tmp/state.json", message: "missing")
        appState.displayError = .RpcCallFailed(method: "pairing.qr", message: "boom")
        appState.isShowingQr = true
        appState.phase = .running

        appState.beginBoot()

        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.relayLinkSubscription)
        XCTAssertNil(appState.peerSubscription)
        XCTAssertEqual(appState.relayLink, .disconnected)
        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.bootError)
        XCTAssertNil(appState.displayError)
        XCTAssertFalse(appState.isShowingQr)
        XCTAssertFalse(appState.canShowQr)
        XCTAssertFalse(appState.canForgetPeer)
        XCTAssertEqual(appState.phase, .awaitingConfig)
    }

    @MainActor
    func testFinishBootPublishesStateAndDerivedFlags() {
        let appState = AppState()
        let daemon = MockDaemon(currentRelayLink: .connected, currentPeer: .unpaired)

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil
        )

        XCTAssertEqual(appState.phase, .running)
        XCTAssertEqual(appState.relayLink, .connected)
        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertTrue(appState.canShowQr)
        XCTAssertFalse(appState.canForgetPeer)
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.bootError)
        XCTAssertNil(appState.displayError)
    }

    @MainActor
    func testFailBootSetsPhaseAndPreservesError() {
        let appState = AppState()
        let relayLinkSub = MockSubscription()
        let peerSub = MockSubscription()

        appState.daemon = MockDaemon()
        appState.relayLinkSubscription = relayLinkSub
        appState.peerSubscription = peerSub
        appState.relayLink = .connecting(attempt: 2)
        appState.peer = .pairing
        appState.isShowingQr = true

        let error = MinosError.CfAuthFailed(message: "Cloudflare denied")
        appState.failBoot(with: error)

        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
        XCTAssertEqual(appState.phase, .bootFailed)
        XCTAssertEqual(appState.bootError, error)
        XCTAssertNil(appState.daemon)
        XCTAssertEqual(appState.relayLink, .disconnected)
        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertFalse(appState.isShowingQr)
    }

    @MainActor
    func testRelayLinkObserverPushUpdatesState() async {
        let (appState, daemon) = AppStateFixtures.runningState()

        daemon.emitRelayLink(.connecting(attempt: 1))
        await AppStateFixtures.drainMainActor()

        XCTAssertEqual(appState.relayLink, .connecting(attempt: 1))
        // Peer axis must NOT have been touched by a link transition.
        XCTAssertEqual(appState.peer, .unpaired)
    }

    @MainActor
    func testPeerObserverPushTracksPairedAndUnpaired() async {
        let (appState, daemon) = AppStateFixtures.runningState()
        let did = "00000000-0000-0000-0000-000000000042"

        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        await AppStateFixtures.drainMainActor()

        XCTAssertEqual(appState.peer, .paired(peerId: did, peerName: "iPhone", online: true))
        XCTAssertEqual(appState.trustedDevice?.deviceId, did)

        daemon.emitPeer(.unpaired)
        await AppStateFixtures.drainMainActor()

        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertNil(appState.trustedDevice)
    }

    @MainActor
    func testReconnectPreservesPeer() async {
        let (appState, daemon) = AppStateFixtures.runningState()
        let did = "00000000-0000-0000-0000-000000000099"

        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        daemon.emitRelayLink(.connecting(attempt: 1))
        await AppStateFixtures.drainMainActor()

        XCTAssertEqual(appState.peer, .paired(peerId: did, peerName: "iPhone", online: true))
        XCTAssertEqual(appState.relayLink, .connecting(attempt: 1))
    }
}

/// Shared scaffolding used by both AppStateBootTests and AppStateActionTests.
enum AppStateFixtures {
    /// Yield twice so any pending @MainActor task scheduled by an
    /// observer closure runs before the next assertion fires.
    @MainActor
    static func drainMainActor() async {
        await Task.yield()
        await Task.yield()
    }

    @MainActor
    static func runningState() -> (AppState, MockDaemon) {
        let daemon = MockDaemon(currentRelayLink: .connected, currentPeer: .unpaired)
        let appState = AppState()

        let relayObserver = RelayLinkObserver { state in
            Task { @MainActor in appState.applyRelayLink(state) }
        }
        let peerObserver = PeerObserver { state in
            Task { @MainActor in appState.applyPeer(state) }
        }
        let relayLinkSub = daemon.subscribeRelayLink(relayObserver)
        let peerSub = daemon.subscribePeer(peerObserver)

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: relayLinkSub,
            peerSubscription: peerSub,
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil
        )
        return (appState, daemon)
    }
}
