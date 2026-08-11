import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart' as core;

/// Top-level navigation surface enum. The router evaluates auth via
/// [decideRootRoute]; online vs offline both map to the shell path.
enum RootRoute {
  /// Pre-stream / refresh-in-flight. Show a spinner so the UI doesn't
  /// flash login during normal cold-launch hydration.
  splash,

  /// No auth (or refresh failed). Route to the email/password screen.
  login,

  /// Authenticated and connected (or transient reconnect). IM shell home.
  shell,

  /// Authenticated but account WS offline. Same shell surface with offline chrome.
  shellOffline,
}

/// Pure decision matrix gating on auth state first, then connection state.
RootRoute decideRootRoute({
  required AuthState authState,
  required core.ConnectionState? connectionState,
  bool hasPersistedPairing = false,
}) {
  return switch (authState) {
    AuthBootstrapping() => RootRoute.splash,
    AuthRefreshing() => RootRoute.splash,
    AuthUnauthenticated() => RootRoute.login,
    AuthRefreshFailed() => RootRoute.login,
    AuthAuthenticated() => switch (connectionState) {
      core.ConnectionState_Connected() => RootRoute.shell,
      core.ConnectionState_Reconnecting() => RootRoute.shell,
      _ => RootRoute.shellOffline,
    },
  };
}
