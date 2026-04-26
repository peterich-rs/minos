import 'package:minos/src/rust/api/minos.dart' as core;

enum RootRoute { pairing, threads }

RootRoute decideRootRoute({
  required core.ConnectionState? connectionState,
  required bool hasPersistedPairing,
}) {
  return switch (connectionState) {
    core.ConnectionState_Connected() => RootRoute.threads,
    core.ConnectionState_Disconnected() when hasPersistedPairing =>
      RootRoute.threads,
    core.ConnectionState_Reconnecting() when hasPersistedPairing =>
      RootRoute.threads,
    _ => RootRoute.pairing,
  };
}
