import AppKit

enum DiagnosticsReveal {
    static func revealTodayLog() async throws {
        let path = try await Task.detached(priority: .utility) {
            try todayLogPath()
        }.value
        let url = URL(fileURLWithPath: path)
        AppLog.info("diagnostics", "Revealing log at \(path)")
        await MainActor.run {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }
}
