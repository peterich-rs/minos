import XCTest

@testable import Minos

/// Gate predicates (`canForgetPeer`) and the forget-peer round-trip.
/// Boot-side scenarios live in `AppStateBootTests`.
final class AppStateTests: XCTestCase {
    func testPeerActivityDateUsesEpochMilliseconds() throws {
        let date = try XCTUnwrap(
            PeerActivityFormatter.date(fromEpochMilliseconds: 1_700_000_000_000)
        )

        XCTAssertEqual(date.timeIntervalSince1970, 1_700_000_000, accuracy: 0.001)

        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        var shanghai = Calendar(identifier: .gregorian)
        shanghai.timeZone = try XCTUnwrap(TimeZone(identifier: "Asia/Shanghai"))

        XCTAssertEqual(utc.component(.hour, from: date), 22)
        XCTAssertEqual(shanghai.component(.hour, from: date), 6)
        XCTAssertNil(PeerActivityFormatter.date(fromEpochMilliseconds: 0))
    }

    // ── Gates ──

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

    // ── Forget round-trip ──

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

        XCTAssertTrue(appState.canForgetPeer)

        await appState.forgetPeer()

        XCTAssertEqual(daemon.forgetPeerCallCount, 1)
        XCTAssertEqual(appState.peer, .unpaired)
        XCTAssertNil(appState.trustedDevice)
    }
}
