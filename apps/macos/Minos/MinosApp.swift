import SwiftUI

@main
struct MinosApp: App {
    @StateObject private var appState: AppState

    init() {
        let initialState = AppState()
        _appState = StateObject(wrappedValue: initialState)

        Task {
            await DaemonBootstrap.bootstrap(initialState)
        }
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarView(appState: appState)
        } label: {
            StatusIcon(
                connectionState: appState.connectionState,
                hasBootError: appState.bootError != nil
            )
            .frame(width: 18, height: 18)
        }
        .menuBarExtraStyle(.window)
    }
}
