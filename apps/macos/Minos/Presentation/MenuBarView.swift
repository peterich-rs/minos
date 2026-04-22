import SwiftUI

struct MenuBarView: View {
    @Bindable var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let displayError = appState.displayError, appState.bootError == nil {
                errorBanner(displayError)
            }

            if let bootError = appState.bootError {
                bootErrorContent(bootError)
            } else if appState.connectionState == nil {
                bootingContent
            } else if appState.trustedDevice != nil {
                pairedContent
            } else {
                unpairedContent
            }
        }
        .padding(14)
        .frame(width: 320)
        .sheet(isPresented: $appState.isQrSheetPresented) {
            QRSheet(appState: appState)
        }
    }

    private var unpairedContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            header(
                title: appState.connectionState?.displayLabel() ?? "正在启动…",
                subtitle: appState.endpointDisplay
            )

            Divider()

            actionButton("显示配对二维码…") {
                Task {
                    await appState.showQr()
                }
            }

            actionButton("在 Finder 中显示今日日志…") {
                Task {
                    await appState.revealTodayLog()
                }
            }

            Divider()

            actionButton("退出 Minos") {
                Task {
                    await appState.shutdown()
                }
            }
        }
    }

    private var pairedContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            header(title: "已配对 · 等待回连", subtitle: appState.trustedDevice?.name)

            Divider()

            actionButton("忘记已配对设备", role: .destructive) {
                Task {
                    await appState.forgetDevice()
                }
            }

            actionButton("在 Finder 中显示今日日志…") {
                Task {
                    await appState.revealTodayLog()
                }
            }

            Divider()

            actionButton("退出 Minos") {
                Task {
                    await appState.shutdown()
                }
            }
        }
    }

    private var bootingContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            header(title: "正在启动…", subtitle: nil)

            Divider()

            actionButton("在 Finder 中显示今日日志…") {
                Task {
                    await appState.revealTodayLog()
                }
            }

            Divider()

            actionButton("退出 Minos") {
                Task {
                    await appState.shutdown()
                }
            }
        }
    }

    private func bootErrorContent(_ bootError: MinosError) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                StatusIcon(connectionState: nil, hasBootError: true)
                Text("Minos · 启动失败")
                    .font(.headline)
            }

            Text(bootError.userMessage(lang: .zh))
                .font(.subheadline)

            DisclosureGroup("详情") {
                Text(bootError.technicalDetails)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
            }

            Button("重试") {
                Task {
                    await DaemonBootstrap.bootstrap(appState)
                }
            }
            .keyboardShortcut(.defaultAction)

            Divider()

            actionButton("在 Finder 中显示今日日志…") {
                Task {
                    await appState.revealTodayLog()
                }
            }

            actionButton("退出 Minos") {
                Task {
                    await appState.shutdown()
                }
            }
        }
    }

    private func header(title: String, subtitle: String?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                StatusIcon(
                    connectionState: appState.connectionState,
                    hasBootError: appState.bootError != nil
                )
                Text("Minos")
                    .font(.headline)
            }

            Text(title)
                .font(.subheadline.weight(.medium))

            if let subtitle {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func errorBanner(_ error: MinosError) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            Text(error.userMessage(lang: .zh))
                .font(.caption)
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.orange.opacity(0.14))
        )
    }

    private func actionButton(
        _ title: String,
        role: ButtonRole? = nil,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            Text(title)
                .foregroundStyle(role == .destructive ? Color.red : Color.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
