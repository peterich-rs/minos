import SwiftUI

/// First-run onboarding shown when no Cloudflare Service Token credentials
/// are present (neither in env vars nor in Keychain). Saving writes both
/// halves to the Keychain via `KeychainRelayConfig` and triggers a fresh
/// daemon bootstrap so the menubar transitions awaitingConfig → running.
///
/// Presented as a real top-level `Window` scene (see `MinosApp`), NOT as
/// a `.sheet` inside the MenuBarExtra popover — the popover's auto-
/// dismiss on focus change made the TextFields unusable otherwise.
///
/// Plan 05 Phase J.1.
struct OnboardingSheet: View {
    @Bindable var appState: AppState
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var clientId: String = ""
    @State private var clientSecret: String = ""
    @State private var saving: Bool = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("配置 Cloudflare Access Service Token")
                .font(.headline)
            Text(
                "首次使用 Minos 需要 Service Token 才能连接后端。请在 Cloudflare Zero Trust 控制台生成后粘贴下方。"
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 8) {
                Text("Client ID").font(.subheadline)
                TextField("", text: $clientId, prompt: Text("xxxxxxxxxx.access"))
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Client Secret").font(.subheadline)
                SecureField("", text: $clientSecret, prompt: Text("paste from dashboard"))
                    .textFieldStyle(.roundedBorder)
            }

            if let error {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            HStack {
                Spacer()
                Button("保存") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(clientId.isEmpty || clientSecret.isEmpty || saving)
            }
        }
        .padding(24)
        .frame(width: 420)
    }

    private func save() {
        saving = true
        do {
            try KeychainRelayConfig.write(
                .init(clientId: clientId, clientSecret: clientSecret)
            )
            dismissWindow(id: WindowID.onboarding)
            Task { await DaemonBootstrap.bootstrap(appState) }
        } catch {
            self.error = "保存失败：\(error.localizedDescription)"
        }
        saving = false
    }
}
