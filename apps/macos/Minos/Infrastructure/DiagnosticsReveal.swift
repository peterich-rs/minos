import AppKit
import OSLog

enum DiagnosticsReveal {
    private static let logger = Logger(subsystem: "ai.minos.macos", category: "diagnostics")

    static func revealTodayLog() async throws {
        let path = try await Task.detached(priority: .utility) {
            try todayLogPath()
        }.value
        let url = URL(fileURLWithPath: path)
        logger.info("Revealing log at \(path, privacy: .public)")
        await MainActor.run {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }
}
