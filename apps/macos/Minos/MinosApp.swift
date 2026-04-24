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
                link: appState.relayLink,
                peer: appState.peer,
                hasBootError: appState.bootError != nil
            )
            .frame(width: 18, height: 18)
        }
        .menuBarExtraStyle(.window)

        // Onboarding + Settings live in real top-level Window scenes,
        // NOT inside the MenuBarExtra(.window) popover. SwiftUI's popover
        // style auto-dismisses on any focus change (including when a
        // TextField inside a nested `.sheet` grabs first-responder), so
        // presenting the credential forms from within the popover makes
        // them unusable — the popover closes the moment the user clicks
        // into the input field. Real Windows have their own key-window
        // state and escape that trap.
        Window("Minos · 首次配置", id: WindowID.onboarding) {
            OnboardingSheet(appState: appState)
        }
        .windowResizability(.contentSize)
        .defaultSize(width: 420, height: 280)

        Window("Minos · Relay 设置", id: WindowID.settings) {
            SettingsSheet(appState: appState)
        }
        .windowResizability(.contentSize)
        .defaultSize(width: 420, height: 220)
    }
}

/// Stable IDs for the two auxiliary windows so openWindow / dismissWindow
/// stay in sync between the Scene declarations above and the call sites
/// in `MenuBarView`, `OnboardingSheet`, `SettingsSheet`.
enum WindowID {
    static let onboarding = "ai.minos.macos.window.onboarding"
    static let settings = "ai.minos.macos.window.settings"
}
