import AppKit
import CoreImage
import Foundation

enum QRCodeRenderer {
    private static let context = CIContext()

    static func image(for payload: QrPayload, dimension: CGFloat = 240) -> NSImage? {
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

    private static func payloadData(for payload: QrPayload) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [
                "host": payload.host,
                "name": payload.name,
                "port": Int(payload.port),
                "token": payload.token,
                "v": Int(payload.v)
            ],
            options: [.sortedKeys]
        )
    }
}
