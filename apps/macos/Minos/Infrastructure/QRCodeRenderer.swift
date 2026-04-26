import AppKit
import CoreImage
import Foundation

/// Renders a `RelayQrPayload` into an `NSImage` suitable for the menubar
/// pairing sheet. The payload is JSON-encoded with a stable field order
/// so the iPhone scanner reliably parses it (the schema lives in spec
/// §7.2: { v, backend_url, host_display_name, pairing_token,
/// expires_at_ms, cf_access_*? }).
///
/// Plan 05 Phase J.4 — switched from minos_pairing.QrPayload (with
/// host/port/name fields) to the relay-flow shape.
enum QRCodeRenderer {
    private static let context = CIContext()

    static func image(for payload: RelayQrPayload, dimension: CGFloat = 240) -> NSImage? {
        guard let filter = CIFilter(name: "CIQRCodeGenerator") else {
            return nil
        }

        do {
            let data = try payloadData(for: payload)
            filter.setValue(data, forKey: "inputMessage")
            filter.setValue("M", forKey: "inputCorrectionLevel")

            guard let outputImage = filter.outputImage else {
                return nil
            }

            let scaled = outputImage.transformed(
                by: CGAffineTransform(
                    scaleX: dimension / outputImage.extent.width,
                    y: dimension / outputImage.extent.height
                )
            )
            guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else {
                return nil
            }
            return NSImage(cgImage: cgImage, size: NSSize(width: dimension, height: dimension))
        } catch {
            return nil
        }
    }

    static func payloadData(for payload: RelayQrPayload) throws -> Data {
        var object: [String: Any] = [
            "v": Int(payload.v),
            "backend_url": payload.backendUrl,
            "host_display_name": payload.hostDisplayName,
            "pairing_token": payload.pairingToken,
            "expires_at_ms": payload.expiresAtMs
        ]
        if let cfAccessClientId = payload.cfAccessClientId {
            object["cf_access_client_id"] = cfAccessClientId
        }
        if let cfAccessClientSecret = payload.cfAccessClientSecret {
            object["cf_access_client_secret"] = cfAccessClientSecret
        }

        return try JSONSerialization.data(
            withJSONObject: object,
            options: [.sortedKeys]
        )
    }
}
