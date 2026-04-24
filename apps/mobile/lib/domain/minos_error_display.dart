import 'package:minos/src/rust/api/minos.dart';

/// UI-layer helpers for [MinosError]. All user-facing copy is delegated to
/// the Rust-side `kind_message` so the localization table has a single owner.
extension MinosErrorDisplay on MinosError {
  /// Map the sealed-class variant onto the matching [ErrorKind] tag. This is
  /// a pure Dart pattern-match — no Rust call is made.
  ErrorKind get kind => switch (this) {
    MinosError_BindFailed() => ErrorKind.bindFailed,
    MinosError_ConnectFailed() => ErrorKind.connectFailed,
    MinosError_Disconnected() => ErrorKind.disconnected,
    MinosError_PairingTokenInvalid() => ErrorKind.pairingTokenInvalid,
    MinosError_PairingStateMismatch() => ErrorKind.pairingStateMismatch,
    MinosError_DeviceNotTrusted() => ErrorKind.deviceNotTrusted,
    MinosError_StoreIo() => ErrorKind.storeIo,
    MinosError_StoreCorrupt() => ErrorKind.storeCorrupt,
    MinosError_CliProbeTimeout() => ErrorKind.cliProbeTimeout,
    MinosError_CliProbeFailed() => ErrorKind.cliProbeFailed,
    MinosError_RpcCallFailed() => ErrorKind.rpcCallFailed,
    MinosError_Unauthorized() => ErrorKind.unauthorized,
    MinosError_ConnectionStateMismatch() => ErrorKind.connectionStateMismatch,
    MinosError_EnvelopeVersionUnsupported() =>
      ErrorKind.envelopeVersionUnsupported,
    MinosError_PeerOffline() => ErrorKind.peerOffline,
    MinosError_RelayInternal() => ErrorKind.relayInternal,
    MinosError_CfAuthFailed() => ErrorKind.cfAuthFailed,
    MinosError_CodexSpawnFailed() => ErrorKind.codexSpawnFailed,
    MinosError_CodexConnectFailed() => ErrorKind.codexConnectFailed,
    MinosError_CodexProtocolError() => ErrorKind.codexProtocolError,
    MinosError_AgentAlreadyRunning() => ErrorKind.agentAlreadyRunning,
    MinosError_AgentNotRunning() => ErrorKind.agentNotRunning,
    MinosError_AgentNotSupported() => ErrorKind.agentNotSupported,
    MinosError_AgentSessionIdMismatch() => ErrorKind.agentSessionIdMismatch,
  };

  /// Localized user-facing message for this error, looked up via the Rust
  /// `kind_message` free function.
  String userMessage([Lang lang = Lang.zh]) =>
      kindMessage(kind: kind, lang: lang);
}
