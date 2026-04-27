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
    MinosError_BackendInternal() => ErrorKind.backendInternal,
    MinosError_CfAuthFailed() => ErrorKind.cfAuthFailed,
    MinosError_CodexSpawnFailed() => ErrorKind.codexSpawnFailed,
    MinosError_CodexConnectFailed() => ErrorKind.codexConnectFailed,
    MinosError_CodexProtocolError() => ErrorKind.codexProtocolError,
    MinosError_AgentAlreadyRunning() => ErrorKind.agentAlreadyRunning,
    MinosError_AgentNotRunning() => ErrorKind.agentNotRunning,
    MinosError_AgentNotSupported() => ErrorKind.agentNotSupported,
    MinosError_AgentSessionIdMismatch() => ErrorKind.agentSessionIdMismatch,
    MinosError_CfAccessMisconfigured() => ErrorKind.cfAccessMisconfigured,
    MinosError_IngestSeqConflict() => ErrorKind.ingestSeqConflict,
    MinosError_ThreadNotFound() => ErrorKind.threadNotFound,
    MinosError_TranslationNotImplemented() =>
      ErrorKind.translationNotImplemented,
    MinosError_TranslationFailed() => ErrorKind.translationFailed,
    MinosError_PairingQrVersionUnsupported() =>
      ErrorKind.pairingQrVersionUnsupported,
    MinosError_Timeout() => ErrorKind.timeout,
    MinosError_NotConnected() => ErrorKind.notConnected,
    MinosError_RequestDropped() => ErrorKind.requestDropped,
    MinosError_AuthRefreshFailed() => ErrorKind.authRefreshFailed,
    MinosError_EmailTaken() => ErrorKind.emailTaken,
    MinosError_WeakPassword() => ErrorKind.weakPassword,
    MinosError_RateLimited() => ErrorKind.rateLimited,
    MinosError_InvalidCredentials() => ErrorKind.invalidCredentials,
    MinosError_AgentStartFailed() => ErrorKind.agentStartFailed,
    MinosError_PairingTokenExpired() => ErrorKind.pairingTokenExpired,
  };

  /// Localized user-facing message for this error, looked up via the Rust
  /// `kind_message` free function.
  String userMessage([Lang lang = Lang.zh]) =>
      kindMessage(kind: kind, lang: lang);

  /// Dynamic detail captured by the Rust side — the underlying TLS / IO /
  /// HTTP-status string for connect failures, the failed RPC method, etc.
  /// `null` for variants without an attached payload (e.g.
  /// [MinosError_PairingTokenInvalid]). Used by the iOS UI to surface the
  /// "WHY" past the static localized hint.
  String? get detail => switch (this) {
    final MinosError_BindFailed e => '${e.addr}: ${e.message}',
    final MinosError_ConnectFailed e => '${e.url} — ${e.message}',
    final MinosError_Disconnected e => e.reason,
    MinosError_PairingTokenInvalid() => null,
    final MinosError_PairingStateMismatch e => 'state=${e.actual}',
    final MinosError_DeviceNotTrusted e => e.deviceId,
    final MinosError_StoreIo e => '${e.path}: ${e.message}',
    final MinosError_StoreCorrupt e => '${e.path}: ${e.message}',
    final MinosError_CliProbeTimeout e => '${e.bin} after ${e.timeoutMs}ms',
    final MinosError_CliProbeFailed e => '${e.bin}: ${e.message}',
    final MinosError_RpcCallFailed e => '${e.method}: ${e.message}',
    final MinosError_Unauthorized e => e.reason,
    final MinosError_ConnectionStateMismatch e =>
      'expected=${e.expected} actual=${e.actual}',
    final MinosError_EnvelopeVersionUnsupported e => 'v=${e.version}',
    final MinosError_PeerOffline e => e.peerDeviceId,
    final MinosError_BackendInternal e => e.message,
    final MinosError_CfAuthFailed e => e.message,
    final MinosError_CodexSpawnFailed e => e.message,
    final MinosError_CodexConnectFailed e => '${e.url}: ${e.message}',
    final MinosError_CodexProtocolError e => '${e.method}: ${e.message}',
    MinosError_AgentAlreadyRunning() => null,
    MinosError_AgentNotRunning() => null,
    final MinosError_AgentNotSupported e => e.agent.toString(),
    MinosError_AgentSessionIdMismatch() => null,
    final MinosError_CfAccessMisconfigured e => e.reason,
    final MinosError_IngestSeqConflict e => '${e.threadId}@${e.seq}',
    final MinosError_ThreadNotFound e => e.threadId,
    final MinosError_TranslationNotImplemented e => e.agent.toString(),
    final MinosError_TranslationFailed e => '${e.agent}: ${e.message}',
    final MinosError_PairingQrVersionUnsupported e => 'v=${e.version}',
    MinosError_Timeout() => null,
    MinosError_NotConnected() => null,
    MinosError_RequestDropped() => null,
    final MinosError_AuthRefreshFailed e => e.message,
    MinosError_EmailTaken() => null,
    MinosError_WeakPassword() => null,
    final MinosError_RateLimited e => 'retry after ${e.retryAfterS}s',
    MinosError_InvalidCredentials() => null,
    final MinosError_AgentStartFailed e => e.reason,
    MinosError_PairingTokenExpired() => null,
  };
}
