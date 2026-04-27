import 'package:minos/src/rust/api/minos.dart';

/// Thin Dart-only contract around the frb-generated [MobileClient]. Letting
/// the application / presentation layers depend on this protocol (rather than
/// the Rust-owned opaque class) keeps the layers mockable in unit tests.
abstract class MinosCoreProtocol {
  /// Submit a raw QR v2 JSON payload to the Rust core. Completes when the
  /// `Pair` RPC returns; the Rust side persists the minted `DeviceSecret`
  /// in its pairing store before this future resolves.
  Future<void> pairWithQrJson(String qrJson);

  /// Forget the paired backend; drops credentials and tears down the WS.
  Future<void> forgetPeer();

  /// Whether the durable store contains enough state to represent an
  /// already-paired device, even if the current WebSocket is offline.
  Future<bool> hasPersistedPairing();

  /// Paged thread summaries for the paired agent-host.
  Future<ListThreadsResponse> listThreads(ListThreadsParams params);

  /// Translated UI event history for one thread.
  Future<ReadThreadResponse> readThread(ReadThreadParams params);

  /// Hot stream of [ConnectionState] transitions, starting with the current
  /// value.
  Stream<ConnectionState> get connectionStates;

  /// Hot stream of live [UiEventFrame]s fanned out by the backend.
  Stream<UiEventFrame> get uiEvents;

  /// Synchronous snapshot of the current [ConnectionState].
  ConnectionState get currentConnectionState;

  // ---- Auth (Phase 8) ----

  /// Register a new account on the backend. On success the Rust core
  /// surfaces `Authenticated` on [authStates] and starts the WS reconnect
  /// loop.
  Future<AuthSummary> register({
    required String email,
    required String password,
  });

  /// Log into an existing account. Same effect on [authStates] as
  /// [register].
  Future<AuthSummary> login({
    required String email,
    required String password,
  });

  /// Rotate the bearer + refresh tokens. Surfaces `Refreshing` /
  /// `Authenticated` / `RefreshFailed` transitions on [authStates].
  Future<void> refreshSession();

  /// Best-effort agent stop, then revoke the refresh token server-side,
  /// then wipe local auth state. Surfaces `Unauthenticated` on
  /// [authStates].
  Future<void> logout();

  // ---- Agent dispatch (Phase 8) ----

  /// Start a new agent session and deliver `prompt` as the first user
  /// message. Returns the daemon-issued `session_id` (a.k.a. `thread_id`)
  /// and the resolved workspace path.
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  });

  /// Send a follow-up user message to an existing agent session. The
  /// `sessionId` is the same value returned by [startAgent].
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  });

  /// Stop the currently-running agent (if any). Idempotent on the
  /// no-active-session path.
  Future<void> stopAgent();

  // ---- Lifecycle (Phase 8) ----

  /// Mark the app as foregrounded. Resets the WS reconnect backoff so the
  /// next connect attempt happens promptly.
  void notifyForegrounded();

  /// Mark the app as backgrounded. Pauses the reconnect loop so we don't
  /// poke the backend while the OS is freezing the process.
  void notifyBackgrounded();

  /// Hot stream of [AuthStateFrame] transitions. Emits the current
  /// cached frame immediately on subscribe (per Rust watch-channel
  /// semantics), then every subsequent change.
  Stream<AuthStateFrame> get authStates;
}
