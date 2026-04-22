import XCTest

@testable import Minos

final class AppStateTests: XCTestCase {
    @MainActor
    func testBeginBootResetsRuntimeStateAndCancelsExistingSubscription() {
        let appState = AppState()
        let daemon = MockDaemon()
        let subscription = MockSubscription()

        appState.daemon = daemon
        appState.subscription = subscription
        appState.connectionState = .connected
        appState.currentQr = MockDaemon.makeQrPayload(name: "Old QR")
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 123)
        appState.trustedDevice = MockDaemon.makeTrustedDevice(name: "Existing Device")
        appState.bootError = .StoreIo(path: "/tmp/state.json", message: "missing")
        appState.displayError = .RpcCallFailed(method: "pairing.qr", message: "boom")
        appState.isShowingQr = true

        appState.beginBoot()

        XCTAssertEqual(subscription.cancelCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.subscription)
        XCTAssertNil(appState.connectionState)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.bootError)
        XCTAssertNil(appState.displayError)
        XCTAssertFalse(appState.isShowingQr)
        XCTAssertFalse(appState.canShowQr)
        XCTAssertFalse(appState.canForgetDevice)
        XCTAssertNil(appState.endpointDisplay)
    }

    @MainActor
    func testFinishBootPublishesStateAndDerivedFlags() {
        let appState = AppState()
        let daemon = MockDaemon(currentState: .pairing, host: "100.64.0.42", port: 7879)
        let subscription = MockSubscription()

        appState.finishBoot(
            daemon: daemon,
            subscription: subscription,
            connectionState: .pairing,
            trustedDevice: nil
        )

        XCTAssertTrue(appState.canShowQr)
        XCTAssertFalse(appState.canForgetDevice)
        XCTAssertEqual(appState.connectionState, .pairing)
        XCTAssertEqual(appState.endpointDisplay, "100.64.0.42:7879")
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.bootError)
        XCTAssertNil(appState.displayError)
    }

    @MainActor
    func testShowQrStoresPayloadAndMarksShowingQr() async throws {
        let expectedQr = MockDaemon.makeQrPayload(host: "100.64.0.55", port: 7880, name: "Office Mac")
        let daemon = MockDaemon(pairingQrResult: .success(expectedQr))
        let appState = AppState()

        appState.finishBoot(
            daemon: daemon,
            subscription: MockSubscription(),
            connectionState: .pairing,
            trustedDevice: nil
        )

        await appState.showQr()

        XCTAssertEqual(daemon.pairingQrCallCount, 1)
        XCTAssertEqual(appState.currentQr, expectedQr)
        XCTAssertTrue(appState.isShowingQr)
        XCTAssertNil(appState.displayError)

        let generatedAt = try XCTUnwrap(appState.currentQrGeneratedAt)
        let expiresAt = try XCTUnwrap(appState.currentQrExpiresAt)
        XCTAssertEqual(expiresAt.timeIntervalSince(generatedAt), 300, accuracy: 0.001)
    }

    @MainActor
    func testForgetDeviceAfterConfirmationClearsPairedState() async {
        let trustedDevice = MockDaemon.makeTrustedDevice()
        let daemon = MockDaemon(currentState: .connected, currentTrustedDevice: trustedDevice)
        let appState = AppState(forgetConfirmation: { _ in true })

        appState.finishBoot(
            daemon: daemon,
            subscription: MockSubscription(),
            connectionState: .connected,
            trustedDevice: trustedDevice
        )
        appState.currentQr = MockDaemon.makeQrPayload()
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 456)
        appState.isShowingQr = true

        XCTAssertFalse(appState.canShowQr)
        XCTAssertTrue(appState.canForgetDevice)
        XCTAssertNil(appState.endpointDisplay)

        await appState.forgetDevice()

        XCTAssertEqual(daemon.forgetDeviceCalls, [trustedDevice.deviceId])
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertFalse(appState.isShowingQr)
    }

    @MainActor
    func testForgetDeviceDoesNothingWhenConfirmationIsRejected() async {
        let trustedDevice = MockDaemon.makeTrustedDevice()
        let daemon = MockDaemon(currentState: .connected, currentTrustedDevice: trustedDevice)
        let appState = AppState(forgetConfirmation: { _ in false })

        appState.finishBoot(
            daemon: daemon,
            subscription: MockSubscription(),
            connectionState: .connected,
            trustedDevice: trustedDevice
        )

        await appState.forgetDevice()

        XCTAssertTrue(daemon.forgetDeviceCalls.isEmpty)
        XCTAssertEqual(appState.trustedDevice, trustedDevice)
        XCTAssertTrue(appState.canForgetDevice)
    }

    @MainActor
    func testFailBootClearsRuntimeStateAndStoresBootError() {
        let appState = AppState()
        let subscription = MockSubscription()

        appState.daemon = MockDaemon()
        appState.subscription = subscription
        appState.connectionState = .reconnecting(attempt: 2)
        appState.currentQr = MockDaemon.makeQrPayload()
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 789)
        appState.trustedDevice = MockDaemon.makeTrustedDevice()
        appState.isShowingQr = true

        let error = MinosError.BindFailed(addr: "tailscale", message: "no 100.x IP")
        appState.failBoot(with: error)

        XCTAssertEqual(subscription.cancelCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.subscription)
        XCTAssertNil(appState.connectionState)
        XCTAssertNil(appState.trustedDevice)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertFalse(appState.isShowingQr)
        XCTAssertEqual(appState.bootError, error)
    }

    @MainActor
    func testShutdownStopsDaemonCancelsSubscriptionAndTerminates() async {
        let daemon = MockDaemon(currentState: .connected)
        let subscription = MockSubscription()
        var terminateCallCount = 0
        let appState = AppState(terminator: { terminateCallCount += 1 })

        appState.finishBoot(
            daemon: daemon,
            subscription: subscription,
            connectionState: .connected,
            trustedDevice: nil
        )
        appState.currentQr = MockDaemon.makeQrPayload()
        appState.currentQrGeneratedAt = Date(timeIntervalSince1970: 999)
        appState.isShowingQr = true

        await appState.shutdown()

        XCTAssertEqual(daemon.stopCallCount, 1)
        XCTAssertEqual(subscription.cancelCallCount, 1)
        XCTAssertEqual(terminateCallCount, 1)
        XCTAssertNil(appState.daemon)
        XCTAssertNil(appState.subscription)
        XCTAssertNil(appState.currentQr)
        XCTAssertNil(appState.currentQrGeneratedAt)
        XCTAssertFalse(appState.isShowingQr)
    }
}
