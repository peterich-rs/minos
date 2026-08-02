import XCTest

@testable import Minos

/// Forget / shutdown / termination coverage split out of `AppStateTests`
/// so each XCTestCase stays under the swiftlint type-body-length cap.
final class AppStateLifecycleTests: XCTestCase {
    private actor StopGate {
        private var continuation: CheckedContinuation<Void, Never>?
        private var isReleased = false

        func waitUntilReleased() async {
            if isReleased {
                return
            }
            await withCheckedContinuation { continuation in
                self.continuation = continuation
            }
        }

        func release() {
            isReleased = true
            continuation?.resume()
            continuation = nil
        }
    }

    @MainActor
    func testForgetPeerDeviceRemovesOnlyTargetedPeerRow() async {
        let first = MockDaemon.makePeerSummary(
            deviceId: "00000000-0000-0000-0000-000000000901",
            deviceName: "Alice iPhone",
            accountEmail: "alice@example.com",
            pairedAtMs: 100,
            lastActiveAtMs: 200,
            online: false
        )
        let second = MockDaemon.makePeerSummary(
            deviceId: "00000000-0000-0000-0000-000000000902",
            deviceName: "Bob iPhone",
            accountEmail: "bob@example.com",
            pairedAtMs: 300,
            lastActiveAtMs: 400,
            online: true
        )
        let daemon = MockDaemon(
            currentRelayLink: .connected,
            currentPeer: .paired(
                peerId: second.mobileDeviceId,
                peerName: second.mobileDeviceName,
                online: true
            ),
            currentPeers: [second, first]
        )
        let appState = AppState(forgetConfirmation: { _ in true })
        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: MockSubscription(),
            peerSubscription: MockSubscription(),
            relayLink: .connected,
            peer: .paired(
                peerId: second.mobileDeviceId,
                peerName: second.mobileDeviceName,
                online: true
            ),
            trustedDevice: nil,
            peers: [second, first]
        )

        await appState.forgetPeerDevice(second)

        XCTAssertEqual(daemon.forgetPeerDeviceCalls, [second.mobileDeviceId])
        XCTAssertEqual(appState.peers, [first])
        XCTAssertEqual(
            appState.peer,
            .paired(
                peerId: first.mobileDeviceId,
                peerName: first.mobileDeviceName,
                online: first.online
            )
        )
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

        await appState.shutdown()

        XCTAssertEqual(daemon.stopCallCount, 1)
        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
        XCTAssertEqual(terminateCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.relayLinkSubscription)
        XCTAssertNil(appState.peerSubscription)
    }

    @MainActor
    func testTerminationControllerRunsShutdownOnceForRepeatedTerminateRequests() async {
        let daemon = MockDaemon(currentRelayLink: .connected, currentPeer: .unpaired)
        let relayLinkSub = MockSubscription()
        let peerSub = MockSubscription()
        let gate = StopGate()
        daemon.stopHook = { await gate.waitUntilReleased() }
        let appState = AppState(terminator: {})

        appState.finishBoot(
            daemon: daemon,
            relayLinkSubscription: relayLinkSub,
            peerSubscription: peerSub,
            relayLink: .connected,
            peer: .unpaired,
            trustedDevice: nil
        )

        let controller = AppTerminationController()
        controller.bind(appState: appState)

        var replies: [Bool] = []
        let first = controller.applicationShouldTerminate {
            replies.append($0)
        }
        // Unstructured Task may not hit stop() after a single yield on CI load.
        // Wait until shutdown has started (stop is in-flight, blocked on gate).
        for _ in 0..<100 where daemon.stopCallCount == 0 {
            await AppStateFixtures.drainMainActor()
        }
        let second = controller.applicationShouldTerminate {
            replies.append($0)
        }

        XCTAssertEqual(first, .terminateLater)
        XCTAssertEqual(second, .terminateLater)
        XCTAssertEqual(
            daemon.stopCallCount,
            1,
            "shutdown must start exactly once before the gate is released"
        )
        XCTAssertTrue(replies.isEmpty, "reply must wait until stop finishes")

        await gate.release()
        for _ in 0..<100 where replies.isEmpty {
            await AppStateFixtures.drainMainActor()
        }

        XCTAssertEqual(replies, [true])
        XCTAssertEqual(relayLinkSub.cancelCallCount, 1)
        XCTAssertEqual(peerSub.cancelCallCount, 1)
        XCTAssertNil(appState.daemon)
    }
}
