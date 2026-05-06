import SwiftUI

enum AppLog {
    static func debug(_ category: String, _ message: @autoclosure () -> String) {
        swiftLogDebug(category: category, message: message())
    }

    static func info(_ category: String, _ message: @autoclosure () -> String) {
        swiftLogInfo(category: category, message: message())
    }

    static func warn(_ category: String, _ message: @autoclosure () -> String) {
        swiftLogWarn(category: category, message: message())
    }

    static func error(_ category: String, _ message: @autoclosure () -> String) {
        swiftLogError(category: category, message: message())
    }
}

@MainActor
final class AppTerminationController {
    private var appState: AppState?
    private var shutdownTask: Task<Void, Never>?

    func bind(appState: AppState) {
        self.appState = appState
    }

    func applicationShouldTerminate(
        reply: @escaping @MainActor (Bool) -> Void
    ) -> NSApplication.TerminateReply {
        guard let appState else {
            return .terminateNow
        }
        guard shutdownTask == nil else {
            return .terminateLater
        }

        shutdownTask = Task {
            await finishTermination(appState: appState, reply: reply)
        }
        return .terminateLater
    }

    private func finishTermination(
        appState: AppState,
        reply: @escaping @MainActor (Bool) -> Void
    ) async {
        await appState.shutdownForTermination()
        reply(true)
        shutdownTask = nil
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    static let terminationController = AppTerminationController()

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        Self.terminationController.applicationShouldTerminate { shouldTerminate in
            sender.reply(toApplicationShouldTerminate: shouldTerminate)
        }
    }
}

@main
struct MinosApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var appState: AppState

    private static var isRunningTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    init() {
        let initialState = AppState()
        _appState = State(initialValue: initialState)
        AppDelegate.terminationController.bind(appState: initialState)

        if !Self.isRunningTests {
            Task {
                await DaemonBootstrap.bootstrap(initialState)
            }
        }
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarView(appState: appState)
        } label: {
            StatusIcon(
                link: appState.relayLink,
                peer: appState.peer,
                hasBootError: appState.bootError != nil
            )
            .frame(width: 18, height: 18)
        }
        .menuBarExtraStyle(.window)
    }
}
