import SwiftUI

struct QRSheet: View {
    @ObservedObject var appState: AppState

    var body: some View {
        VStack(spacing: 18) {
            if let pairingPayload = appState.currentQr {
                qrContent(pairingPayload)
            } else {
                unavailableContent
            }
        }
        .padding(24)
        .frame(width: 360)
    }

    @ViewBuilder
    private func qrContent(_ pairingPayload: QrPayload) -> some View {
        if let generatedAt = appState.currentQrGeneratedAt {
            TimelineView(.periodic(from: generatedAt, by: 1)) { context in
                content(
                    pairingPayload: pairingPayload,
                    isExpired: context.date.timeIntervalSince(generatedAt) >= 300
                )
            }
        } else {
            content(pairingPayload: pairingPayload, isExpired: false)
        }
    }

    private var unavailableContent: some View {
        VStack(spacing: 12) {
            Text("二维码不可用")
                .font(.headline)
            Button("关闭") {
                appState.dismissQrSheet()
            }
        }
    }

    private func content(pairingPayload: QrPayload, isExpired: Bool) -> some View {
        VStack(spacing: 16) {
            qrPanel(pairingPayload: pairingPayload, isExpired: isExpired)

            Text("有效期 5 分钟 · 在手机上扫描")
                .font(.headline)
            Text("\(pairingPayload.host):\(pairingPayload.port)")
                .font(.caption)
                .foregroundStyle(.secondary)

            HStack(spacing: 10) {
                Button("重新生成") {
                    Task {
                        await appState.regenerateQr()
                    }
                }
                .keyboardShortcut(.defaultAction)

                Button("关闭") {
                    appState.dismissQrSheet()
                }
            }
        }
    }

    private func qrPanel(pairingPayload: QrPayload, isExpired: Bool) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 18)
                .fill(Color(NSColor.windowBackgroundColor))
                .overlay(
                    RoundedRectangle(cornerRadius: 18)
                        .strokeBorder(Color.secondary.opacity(0.18))
                )

            qrImageContent(pairingPayload: pairingPayload)

            if isExpired {
                RoundedRectangle(cornerRadius: 18)
                    .fill(.ultraThinMaterial)
                VStack(spacing: 6) {
                    Text("二维码已过期")
                        .font(.headline)
                    Text("请重新生成后再用手机扫码")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .frame(width: 240, height: 240)
    }

    @ViewBuilder
    private func qrImageContent(pairingPayload: QrPayload) -> some View {
        if let image = QRCodeRenderer.image(for: pairingPayload) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(20)
        } else {
            VStack(spacing: 8) {
                Image(systemName: "qrcode")
                    .font(.system(size: 48))
                    .foregroundStyle(.secondary)
                Text("二维码生成失败")
                    .foregroundStyle(.secondary)
            }
            .padding(20)
        }
    }
}
