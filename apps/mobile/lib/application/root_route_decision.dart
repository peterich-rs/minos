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

  /// Authenticated but no paired Mac. Route to the QR scanner.
  pairing,

  /// Authenticated + paired + WS up (or transient reconnect). Route to
  /// the chat list.
  threadList,

  /// Authenticated + paired but the Mac peer is offline / WS torn down.
  /// Same surface as [threadList] visually but expected to render a
  /// "Mac is offline" banner and disable the input bar.
  threadListMacOffline,
}

/// Pure decision matrix gating on auth state first, then pairing
/// presence, then connection state. Inputs come from the providers in
/// `application/`.
RootRoute decideRootRoute({
  required AuthState authState,
  required core.ConnectionState? connectionState,
  required bool hasPersistedPairing,
}) {
  return switch (authState) {
    AuthBootstrapping() => RootRoute.splash,
    AuthRefreshing() => RootRoute.splash,
    AuthUnauthenticated() => RootRoute.login,
    AuthRefreshFailed() => RootRoute.login,
    AuthAuthenticated() when !hasPersistedPairing => RootRoute.pairing,
    AuthAuthenticated() => switch (connectionState) {
      core.ConnectionState_Connected() => RootRoute.threadList,
      core.ConnectionState_Reconnecting() => RootRoute.threadList,
      _ => RootRoute.threadListMacOffline,
    },
  };
}
