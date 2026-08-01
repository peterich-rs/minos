import 'package:minos/src/rust/api/minos.dart';

/// Thin Dart-only contract around the frb-generated [MobileClient]. Letting
/// the application / presentation layers depend on this protocol (rather than
/// the Rust-owned opaque class) keeps the layers mockable in unit tests.
abstract class MinosCoreProtocol {
  /// Unlink a host installation from the account and clear local active host
  /// when it matches (`POST /v1/hosts/unlink`).
  Future<void> forgetHost(String hostDeviceId);

  /// Linked hosts for the current account (`GET /v1/hosts`).
  Future<List<HostSummaryDto>> listPairedHosts();

  /// `host_device_id` of the Mac currently selected as the routing target,
  /// or `null` when no active Mac is set.
  Future<String?> activeHost();

  /// Set the routing target. Subsequent host commands go to this Mac.
  Future<void> setActiveHost(String hostDeviceId);

  /// Whether the durable store has enough auth state for a cold-start resume.
  Future<bool> hasPersistedPairing();

  /// Optional display label for the active host (local preference).
  Future<String?> peerDisplayName();

  /// Persist the active host display name. Pass `null` or empty to clear.
  Future<void> setPeerDisplayName(String? name);

  Future<MyProfileResponse> myProfile();

  Future<MyProfileResponse> setMinosId({required String minosId});

  Future<List<UserSummary>> searchUsers({required String minosId});

  Future<FriendRequestSummary> createFriendRequest({
    required String targetMinosId,
  });

  Future<FriendRequestsResponse> friendRequests();

  Future<FriendRequestSummary> acceptFriendRequest({required String requestId});

  Future<FriendRequestSummary> rejectFriendRequest({required String requestId});

  Future<FriendsResponse> friends();

  Future<AgentSummary> registerAgent({
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
    String? workspacePath,
  });

  Future<AgentSummary> updateAgent({
    required String agentId,
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
    String? workspacePath,
  });

  Future<ListAgentsResponse> listAgents();

  Future<ConversationsResponse> conversations();

  Future<void> deleteConversation({required String conversationId});

  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  });

  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  });

  Future<void> addGroupMember({
    required String conversationId,
    required String memberAccountId,
  });

  Future<void> removeGroupMember({
    required String conversationId,
    required String memberAccountId,
  });

  Future<ConversationMembersResponse> conversationMembers({
    required String conversationId,
  });

  Future<ConversationAgentMembersResponse> listConversationAgents({
    required String conversationId,
  });

  Future<void> addAgentToConversation({
    required String conversationId,
    required String agentId,
  });

  Future<void> removeAgentFromConversation({
    required String conversationId,
    required String agentId,
  });

  Future<ConversationReadResponse> markConversationRead({
    required String conversationId,
  });

  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int? beforeTsMs,
    int limit = 50,
  });

  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
  });

  Future<ChatMessageSummary> recallChatMessage({
    required String conversationId,
    required String messageId,
  });

  /// Paged thread summaries for the paired agent-host.
  Future<ListSessionsResponse> listThreads(ListSessionsParams params);

  /// Recent agent sessions, optionally scoped to one social conversation.
  Future<List<AgentSessionSummaryDto>> listAgentSessions({
    String? conversationId,
    int limit = 20,
  });

  /// Subscribe the live WebSocket to one agent session topic.
  Future<void> subscribeAgentSession({required String sessionId});

  /// Translated UI event history for one session.
  Future<ReadSessionResponse> readThread(ReadSessionParams params);

  // ---- Projects (Phase P) ----

  /// Create a new project on the daemon.
  Future<CreateProjectResponse> createProject({
    required String name,
    required String workspaceSlug,
    String? workspacePath,
  });

  /// List all projects on the daemon.
  Future<ListProjectsResponse> listProjects();

  /// Update a project's name.
  Future<void> updateProject({required String projectId, required String name});

  /// Delete a project.
  Future<void> deleteProject({required String projectId});

  /// List sessions within a project.
  Future<ListProjectSessionsResponse> listProjectThreads({
    required String projectId,
    int limit = 50,
    int? beforeTsMs,
  });

  /// Hot stream of [ConnectionState] transitions, starting with the current
  /// value.
  Stream<ConnectionState> get connectionStates;

  /// Hot stream of live [UiEventFrame]s fanned out by the backend.
  Stream<UiEventFrame> get uiEvents;

  /// Hot stream of live [SocialEventFrame]s fanned out by the backend.
  Stream<SocialEventFrame> get socialEvents;

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

  /// Exchange a Supabase Auth access token for a Minos session (cloud IdP).
  Future<AuthSummary> loginWithSupabase({required String supabaseAccessToken});

  /// Rotate the bearer + refresh tokens. Surfaces `Refreshing` /
  /// `Authenticated` / `RefreshFailed` transitions on [authStates].
  Future<void> refreshSession();

  /// Best-effort agent stop, then revoke the refresh token server-side,
  /// then wipe local auth state. Surfaces `Unauthenticated` on
  /// [authStates].
  Future<void> logout();

  // ---- Agent dispatch (Phase 8) ----

  /// Send a follow-up user message to an existing agent session. The
  /// `sessionId` is the session/session identifier.
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  });

  /// Pause an in-flight turn while keeping the session resumable.
  Future<void> interruptThread({required String sessionId});

  /// Close an agent session by its `session_id`. Replaces the pre-Phase-C
  /// `stop_agent()` surface — the multi-thread `AgentManager` keys lifecycle
  /// operations on `session_id` rather than implicitly on the single active
  /// session. Idempotent on the daemon side; calling for an already-closed
  /// thread is a benign no-op.
  Future<void> closeThread({required String sessionId});

  /// Permanently delete a thread. Used exclusively for the swipe-to-delete
  /// gesture in the session list. Semantically identical to [closeThread] on
  /// the wire, but named distinctly so call-sites express intent: interrupt
  /// pauses a session for later resume, while delete is a permanent close.
  Future<void> deleteThread({required String sessionId});

  /// Detect CLI agents available on the paired runtime.
  Future<List<AgentDescriptor>> listClis();

  /// Scan host-side skills for the selected runtime host.
  Future<ListHostSkillsResponse> listHostSkills({
    String? hostDeviceId,
    bool forceReload = true,
  });

  /// List selectable workspace directories on the selected runtime host.
  Future<ListHostWorkspacesResponse> listHostWorkspaces({
    String? hostDeviceId,
    String? root,
    int limit = 100,
  });

  /// Enable or disable one host-side skill by path.
  Future<WriteHostSkillConfigResponse> writeHostSkillConfig({
    String? hostDeviceId,
    required String path,
    required bool enabled,
  });

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

  /// Send an approval decision (accept/decline) for a pending approval
  /// request. The [requestId] must match the original request's id, and
  /// [sessionId] identifies the session the approval belongs to. The
  /// [decision] is a JSON-encodable value matching the expected response
  /// shape for the approval variant (command execution, file change, or
  /// permissions).
  Future<void> sendApprovalDecision({
    required String requestId,
    required String sessionId,
    required Map<String, dynamic> decision,
  });

  /// Submit answers for a pending opencode question request.
  Future<void> respondOpencodeQuestion({
    required String sessionId,
    required String questionId,
    required List<List<String>> answers,
  });

  /// Re-open the WS using the durable pairing snapshot already loaded
  /// into the Rust core. Idempotent: a no-op when [currentConnectionState]
  /// is already `Connected`, and an error when no pairing snapshot exists.
  ///
  /// Called by `AuthController` on the first `Authenticated` transition
  /// (Phase 8.9) so the WS reconnect loop only spawns under an
  /// authenticated session.
  Future<void> resumePersistedSession();
}
