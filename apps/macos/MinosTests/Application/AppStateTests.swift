import XCTest

@testable import Minos

/// Plan 05 Phase K.2: rewritten for the dual-axis state model.
final class AppStateTests: XCTestCase {
    // ── beginBoot / failBoot / shutdown ──

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

    // ── Observer push paths (the main reason the dual-axis split exists) ──

    @MainActor
    func testRelayLinkObserverPushUpdatesState() async {
        let (appState, daemon) = makeRunningState()

        daemon.emitRelayLink(.connecting(attempt: 1))
        await drainMainActor()

        XCTAssertEqual(appState.relayLink, .connecting(attempt: 1))
        // Peer axis must NOT have been touched by a link transition.
        XCTAssertEqual(appState.peer, .unpaired)
    }

    @MainActor
    func testPeerObserverPushTracksPairedAndUnpaired() async {
        let (appState, daemon) = makeRunningState()
        let did = "00000000-0000-0000-0000-000000000042"

        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        await drainMainActor()

        XCTAssertEqual(appState.peer, .paired(peerId: did, peerName: "iPhone", online: true))
        XCTAssertEqual(appState.trustedDevice?.deviceId, did)

        daemon.emitPeer(.unpaired)
        await drainMainActor()

        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertNil(appState.trustedDevice)
    }

    @MainActor
    func testReconnectPreservesPeer() async {
        let (appState, daemon) = makeRunningState()
        let did = "00000000-0000-0000-0000-000000000099"

        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        daemon.emitRelayLink(.connecting(attempt: 1))
        await drainMainActor()

        XCTAssertEqual(appState.peer, .paired(peerId: did, peerName: "iPhone", online: true))
        XCTAssertEqual(appState.relayLink, .connecting(attempt: 1))
    }

    // ── canShowQr / canForgetPeer gates ──

    @MainActor
    func testCanShowQrFalseWhenLinkDownEvenIfUnpaired() async {
        let (appState, daemon) = makeRunningState()
        daemon.emitRelayLink(.disconnected)
        await drainMainActor()

        XCTAssertFalse(appState.canShowQr)
    }

    @MainActor
    func testCanForgetPeerFalseWhenLinkDown() async {
        let (appState, daemon) = makeRunningState()
        let did = "00000000-0000-0000-0000-000000000777"
        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        daemon.emitRelayLink(.disconnected)
        await drainMainActor()

        XCTAssertFalse(appState.canForgetPeer)
    }

    @MainActor
    func testCanForgetPeerTrueWhenPairedAndConnected() async {
        let (appState, daemon) = makeRunningState()
        let did = "00000000-0000-0000-0000-000000000888"
        daemon.emitPeer(.paired(peerId: did, peerName: "iPhone", online: true))
        await drainMainActor()

        XCTAssertTrue(appState.canForgetPeer)
    }

    // ── Pairing round-trips ──

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

    // ── Helpers ──

    /// Yield until any pending @MainActor tasks scheduled by observer
    /// callbacks have run. Two yields are enough in practice — one to
    /// release the current main-actor turn so the observer's
    /// `Task { @MainActor in ... }` can be picked up, and a second to
    /// let any chained @MainActor work complete.
    @MainActor
    private func drainMainActor() async {
        await Task.yield()
        await Task.yield()
    }

    @MainActor
    private func makeRunningState() -> (AppState, MockDaemon) {
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
