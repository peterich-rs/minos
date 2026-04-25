import XCTest

@testable import Minos

/// Action-side coverage of `AppState`: gate computed properties (canShowQr /
/// canForgetPeer) and the round-trip pairing/forget paths. Boot-side
/// scenarios live in `AppStateBootTests`.
final class AppStateTests: XCTestCase {
    // ── Gates ──

    @MainActor
    func testCanShowQrFalseWhenLinkDownEvenIfUnpaired() async {
        let (appState, daemon) = AppStateFixtures.runningState()
        daemon.emitRelayLink(.disconnected)
        await AppStateFixtures.drainMainActor()

        XCTAssertFalse(appState.canShowQr)
    }

    @MainActor
    func testCanForgetPeerFalseWhenLinkDown() async {
        let (appState, daemon) = AppStateFixtures.runningState()
        let did = "00000000-0000-0000-0000-000000000777"
        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        daemon.emitRelayLink(.disconnected)
        await AppStateFixtures.drainMainActor()

        XCTAssertFalse(appState.canForgetPeer)
    }

    @MainActor
    func testCanForgetPeerTrueWhenPairedAndConnected() async {
        let (appState, daemon) = AppStateFixtures.runningState()
        let did = "00000000-0000-0000-0000-000000000888"
        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        await AppStateFixtures.drainMainActor()

        XCTAssertTrue(appState.canForgetPeer)
    }

    // ── QR / forget round-trips ──

    @MainActor
    func testShowQrStoresPayloadAndMarksShowingQr() async throws {
        let expected = MockDaemon.makeQrPayload(macDisplayName: "Office Mac")
        let daemon = MockDaemon(
            currentRelayLink: .connected,
            currentPeer: .unpaired,
            pairingQrResult: .success(expected)
        )
        let appState = AppState()

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil
        )

        await appState.showQr()

        XCTAssertEqual(daemon.pairingQrCallCount, 1)
        XCTAssertEqual(appState.currentQr, expected)
        XCTAssertTrue(appState.isShowingQr)
        XCTAssertNil(appState.displayError)

        let generatedAt = try XCTUnwrap(appState.currentQrGeneratedAt)
        let expiresAt = try XCTUnwrap(appState.currentQrExpiresAt)
        XCTAssertEqual(expiresAt.timeIntervalSince(generatedAt), 300, accuracy: 0.001)
    }

    @MainActor
    func testForgetPeerSuccessClearsLocalAndCallsMock() async {
        let trusted = MockDaemon.makeTrustedDevice()
        let daemon = MockDaemon(
            currentRelayLink: .connected,
            currentPeer: .paired(
                peerId: trusted.deviceId,
                peerName: trusted.name,
                online: true
            ),
            currentTrustedDevice: trusted
        )
        let appState = AppState(forgetConfirmation: { _ in true })
        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .paired(peerId: trusted.deviceId, peerName: trusted.name, online: true),
            trustedDevice: trusted
        )
        appState.currentQr = MockDaemon.makeQrPayload()
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 456)
        appState.isShowingQr = true

        XCTAssertTrue(appState.canForgetPeer)
        XCTAssertFalse(appState.canShowQr)

        await appState.forgetPeer()

        XCTAssertEqual(daemon.forgetPeerCallCount, 1)
        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertFalse(appState.isShowingQr)
    }

    @MainActor
    func testForgetPeerDoesNothingWhenConfirmationRejected() async {
        let trusted = MockDaemon.makeTrustedDevice()
        let daemon = MockDaemon(
            currentRelayLink: .connected,
            currentPeer: .paired(
                peerId: trusted.deviceId,
                peerName: trusted.name,
                online: true
            ),
            currentTrustedDevice: trusted
        )
        let appState = AppState(forgetConfirmation: { _ in false })
        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .paired(peerId: trusted.deviceId, peerName: trusted.name, online: true),
            trustedDevice: trusted
        )

        await appState.forgetPeer()

        XCTAssertEqual(daemon.forgetPeerCallCount, 0)
        XCTAssertEqual(appState.trustedDevice, trusted)
        XCTAssertTrue(appState.canForgetPeer)
    }

    @MainActor
    func testShutdownStopsDaemonCancelsBothSubscriptionsAndTerminates() async {
        let daemon = MockDaemon(currentRelayLink: .connected, currentPeer: .unpaired)
        let relayLinkSub = MockSubscription()
        let peerSub = MockSubscription()
        var terminateCallCount = 0
        let appState = AppState(terminator: { terminateCallCount += 1 })

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: relayLinkSub,
            peerSubscription: peerSub,
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil
        )
        appState.currentQr = MockDaemon.makeQrPayload()
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 999)
        appState.isShowingQr = true

        await appState.shutdown()

        XCTAssertEqual(daemon.stopCallCount, 1)
        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
        XCTAssertEqual(terminateCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.relayLinkSubscription)
        XCTAssertNil(appState.peerSubscription)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertFalse(appState.isShowingQr)
    }
}
