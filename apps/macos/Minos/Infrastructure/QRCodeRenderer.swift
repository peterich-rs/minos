import AppKit
import CoreImage
import Foundation

/// Renders a `RelayQrPayload` into an `NSImage` suitable for the menubar
/// pairing sheet. The payload is JSON-encoded with a stable field order
/// so the iPhone scanner reliably parses it (the schema lives in spec
/// §4.2: { v, backend_url, token, mac_display_name }).
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

    private static func payloadData(for payload: RelayQrPayload) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [
                "v": Int(payload.v),
                "backend_url": payload.backendUrl,
                "token": payload.token,
                "mac_display_name": payload.macDisplayName
            ],
            options: [.sortedKeys]
        )
    }
}
