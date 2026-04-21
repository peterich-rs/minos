import AppKit
import OSLog

enum DiagnosticsReveal {
    private static let logger = Logger(subsystem: "ai.minos.macos", category: "diagnostics")

    @MainActor
    static func revealTodayLog() throws {
        let path = try todayLogPath()
        let url = URL(fileURLWithPath: path)
        logger.info("Revealing log at \(path, privacy: .public)")
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }
}
