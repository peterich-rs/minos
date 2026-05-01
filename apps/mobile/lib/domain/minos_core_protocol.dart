import 'package:minos/src/rust/api/minos.dart';

/// Thin Dart-only contract around the frb-generated [MobileClient]. Letting
/// the application / presentation layers depend on this protocol (rather than
/// the Rust-owned opaque class) keeps the layers mockable in unit tests.
abstract class MinosCoreProtocol {
  /// Submit a raw QR v2 JSON payload to the Rust core. Completes when the
  /// `Pair` RPC returns; the Rust side persists the minted device id
  /// in its pairing store before this future resolves.
  Future<void> pairWithQrJson(String qrJson);

  /// Forget a specific paired Mac (by `host_device_id`). After ADR-0020 the
  /// pairing is account-scoped on the server: this drops the
  /// `account_host_pairings` row and tears down the WS to that Mac.
  Future<void> forgetHost(String hostDeviceId);

  /// Paired Mac partners for the current account. Returns an empty list
  /// when no Macs are paired or the WS hasn't synced yet.
  Future<List<HostSummaryDto>> listPairedHosts();

  /// `host_device_id` of the Mac currently selected as the routing target,
  /// or `null` when no active Mac is set.
  Future<String?> activeHost();

  /// Set the routing target. Subsequent `Forward` envelopes will be
  /// `target_device_id`-stamped to this Mac.
  Future<void> setActiveHost(String hostDeviceId);

  /// Whether the durable store contains enough state to represent an
  /// authenticated device, even if the current WebSocket is offline.
  Future<bool> hasPersistedPairing();

  /// Display name of the currently paired peer, sourced from the QR's
  /// `host_display_name` at pair time. Returns `null` when no pairing
  /// is persisted or the name was never recorded.
  Future<String?> peerDisplayName();

  /// Persist the paired peer's display name. Pass `null` or empty to
  /// clear the stored value.
  Future<void> setPeerDisplayName(String? name);

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
  Future<AuthSummary> login({required String email, required String password});

  /// Rotate the bearer + refresh tokens. Surfaces `Refreshing` /
  /// `Authenticated` / `RefreshFailed` transitions on [authStates].
  Future<void> refreshSession();

  /// Best-effort agent stop, then revoke the refresh token server-side,
  /// then wipe local auth state. Surfaces `Unauthenticated` on
  /// [authStates].
  Future<void> logout();

  // ---- Agent dispatch (Phase 8) ----

  /// Start a new agent session. `prompt` is retained so the UI can keep the
  /// typed text visible during `SessionStarting`, but the first user message
  /// must be sent separately via [sendUserMessage]. Returns the daemon-issued
  /// `session_id` (a.k.a. `thread_id`) and the resolved workspace path.
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

  /// Detect CLI agents available on the paired runtime.
  Future<List<AgentDescriptor>> listClis();

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

  /// Re-open the WS using the durable pairing snapshot already loaded
  /// into the Rust core. Idempotent: a no-op when [currentConnectionState]
  /// is already `Connected`, and an error when no pairing snapshot exists.
  ///
  /// Called by `AuthController` on the first `Authenticated` transition
  /// (Phase 8.9) so the WS reconnect loop only spawns under an
  /// authenticated session.
  Future<void> resumePersistedSession();
}
