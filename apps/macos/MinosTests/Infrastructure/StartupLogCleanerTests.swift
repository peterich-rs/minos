import XCTest

@testable import Minos

final class StartupLogCleanerTests: XCTestCase {
    func testAppDirectoriesLogsDirectoryUsesMinosHome() {
        let path = AppDirectories.logsDirectory(env: ["MINOS_HOME": "/tmp/minos-shared"])

        XCTAssertEqual(path.path, "/tmp/minos-shared/logs")
    }

    func testStartupLogCleanerDeletesFilesAndKeepsDirectories() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let logs = root.appendingPathComponent("logs", isDirectory: true)
        let archive = logs.appendingPathComponent("archive", isDirectory: true)
        let xlog = logs.appendingPathComponent("daemon_20260610.xlog")
        let lock = logs.appendingPathComponent("daemon.lock")
        let archivedLog = archive.appendingPathComponent("daemon_20260609.xlog")
        defer { try? FileManager.default.removeItem(at: root) }

        try FileManager.default.createDirectory(at: archive, withIntermediateDirectories: true)
        try Data("current".utf8).write(to: xlog)
        try Data("lock".utf8).write(to: lock)
        try Data("old".utf8).write(to: archivedLog)

        let result = StartupLogCleaner.clearExistingLogs(env: ["MINOS_HOME": root.path])

        XCTAssertEqual(result.logDirectory.path, logs.path)
        XCTAssertEqual(result.deletedCount, 2)
        XCTAssertEqual(result.skippedCount, 1)
        XCTAssertTrue(result.failures.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: xlog.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: lock.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: archive.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: archivedLog.path))
    }
}
