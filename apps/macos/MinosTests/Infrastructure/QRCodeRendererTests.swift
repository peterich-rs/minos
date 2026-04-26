import XCTest

@testable import Minos

final class QRCodeRendererTests: XCTestCase {
    func testPayloadDataUsesMobileQrV2FieldNames() throws {
        let payload = MockDaemon.makeQrPayload(
            backendUrl: "wss://minos.fan-nn.top/devices",
            pairingToken: "pairing-token",
            hostDisplayName: "Office Mac",
            expiresAtMs: 1_700_000_123_000,
            cfAccessClientId: "cf-id",
            cfAccessClientSecret: "cf-secret"
        )

        let data = try QRCodeRenderer.payloadData(for: payload)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )

        XCTAssertEqual(object["v"] as? Int, 2)
        XCTAssertEqual(object["backend_url"] as? String, "wss://minos.fan-nn.top/devices")
        XCTAssertEqual(object["host_display_name"] as? String, "Office Mac")
        XCTAssertEqual(object["pairing_token"] as? String, "pairing-token")
        XCTAssertEqual((object["expires_at_ms"] as? NSNumber)?.int64Value, 1_700_000_123_000)
        XCTAssertEqual(object["cf_access_client_id"] as? String, "cf-id")
        XCTAssertEqual(object["cf_access_client_secret"] as? String, "cf-secret")
        XCTAssertNil(object["token"])
        XCTAssertNil(object["mac_display_name"])
    }
}
