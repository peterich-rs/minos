import SwiftUI

@main
struct MinosApp: App {
    @State private var appState: AppState

    init() {
        let initialState = AppState()
        _appState = State(initialValue: initialState)

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
