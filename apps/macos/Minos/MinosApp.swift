import SwiftUI

@main
struct MinosApp: App {
    @State private var appState: AppState

    private static var isRunningTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    init() {
        let initialState = AppState()
        _appState = State(initialValue: initialState)

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
