import XCTest

@testable import Minos

final class QRCodeRendererTests: XCTestCase {
    func testPayloadDataUsesMobileQrV2FieldNames() throws {
        let payload = MockDaemon.makeQrPayload(
            pairingToken: "pairing-token",
            hostDisplayName: "Office Mac",
            expiresAtMs: 1_700_000_123_000
        )

        let data = try QRCodeRenderer.payloadData(for: payload)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )

        XCTAssertEqual(object["v"] as? Int, 2)
        XCTAssertEqual(object["host_display_name"] as? String, "Office Mac")
        XCTAssertEqual(object["pairing_token"] as? String, "pairing-token")
        XCTAssertEqual((object["expires_at_ms"] as? NSNumber)?.int64Value, 1_700_000_123_000)
        // Backend URL and CF Access tokens no longer travel with the QR; they
        // are compile-time client config in `minos_mobile::build_config`.
        XCTAssertNil(object["backend_url"])
        XCTAssertNil(object["cf_access_client_id"])
        XCTAssertNil(object["cf_access_client_secret"])
        XCTAssertNil(object["token"])
        XCTAssertNil(object["mac_display_name"])
    }
}
