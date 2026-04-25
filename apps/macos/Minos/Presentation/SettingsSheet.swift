import SwiftUI

/// "Relay 设置…" entry point for established users — same fields as
/// onboarding plus a Cancel and a stop-then-rebootstrap on save.
///
/// Surfaces a warning when an env-var override is active because the
/// persisted Keychain values won't take effect until the user unsets
/// the env vars before relaunching.
///
/// Presented as a real top-level `Window` scene (see `MinosApp`), NOT as
/// a `.sheet` inside the MenuBarExtra popover — same reason as
/// `OnboardingSheet`: popover focus semantics break text input.
///
/// Plan 05 Phase J.1.
struct SettingsSheet: View {
    @Bindable var appState: AppState
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var clientId: String = ""
    @State private var clientSecret: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Relay 设置").font(.headline)

            if ProcessInfo.processInfo.environment["CF_ACCESS_CLIENT_ID"] != nil {
                Text("当前有环境变量覆盖生效，本次保存的值在 unset 环境变量之前不会生效。")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }

            TextField("Client ID", text: $clientId, prompt: Text("xxxxxxxxxx.access"))
                .textFieldStyle(.roundedBorder)
            SecureField("Client Secret", text: $clientSecret, prompt: Text("paste from dashboard"))
                .textFieldStyle(.roundedBorder)

            HStack {
                Button("取消") { dismissWindow(id: WindowID.settings) }
                Spacer()
                Button("保存") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(clientId.isEmpty || clientSecret.isEmpty)
            }
        }
        .padding(24)
        .frame(width: 420)
        .onAppear {
            if let creds = KeychainRelayConfig.read() {
                clientId = creds.clientId
                clientSecret = creds.clientSecret
            }
        }
    }

    private func save() {
        try? KeychainRelayConfig.write(.init(clientId: clientId, clientSecret: clientSecret))
        dismissWindow(id: WindowID.settings)
        Task {
            // Stop the running daemon (so the next bootstrap can mint a
            // fresh relay client with the new creds) then re-enter
            // bootstrap. We deliberately do NOT call AppState.shutdown
            // here — that path terminates the app.
            try? await appState.daemon?.stop()
            await DaemonBootstrap.bootstrap(appState)
        }
    }
}
