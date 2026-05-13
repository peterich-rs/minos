import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart' as core;

/// Top-level navigation surface enum. The router watches three providers
/// (auth, connection, persisted-pairing) and routes via [decideRootRoute].
enum RootRoute {
  /// Pre-stream / refresh-in-flight. Show a spinner so the UI doesn't
  /// flash login during normal cold-launch hydration.
  splash,

  /// No auth (or refresh failed). Route to the email/password screen.
  login,

  /// Authenticated + paired + WS up (or transient reconnect). Route to
  /// the project list (new home).
  projectList,

  /// Authenticated + paired but the connection to server is offline / WS
  /// torn down. Same surface as [projectList] visually but expected to render
  /// an offline banner.
  projectListOffline,
}

/// Pure decision matrix gating on auth state first, then connection state.
/// Pairing is now a user-initiated "Add partner" flow from Profile rather
/// than a top-level auth gate.
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
      core.ConnectionState_Connected() => RootRoute.projectList,
      core.ConnectionState_Reconnecting() => RootRoute.projectList,
      _ => RootRoute.projectListOffline,
    },
  };
}
