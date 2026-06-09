import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/infrastructure/frb_external_library.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// The one place in the Dart codebase allowed to import the frb-generated
/// [MobileClient]. Everything above this layer depends on
/// [MinosCoreProtocol] instead.
class MinosCore implements MinosCoreProtocol {
  MinosCore._(this._client, this._secure);

  factory MinosCore.forTesting({
    required MobileClient client,
    required SecurePairingStore secureStore,
  }) => MinosCore._(client, secureStore);

  final MobileClient _client;
  final SecurePairingStore _secure;

  /// Construct and initialize the core. Must be awaited before any other
  /// Riverpod provider reads it.
  static Future<MinosCore> init({
    required String selfName,
    required String logDir,
    SecurePairingStore? secureStore,
  }) async {
    await RustLib.init(externalLibrary: resolveFrbExternalLibrary());
    if (!kIsWeb) {
      await initLogging(logDir: logDir);
    }
    final secure = secureStore ?? SecurePairingStore();
    final client = await resolveClient(
      secure: secure,
      buildFresh: () => MobileClient(selfName: selfName),
      buildFromPersisted: (state) =>
          MobileClient.newWithPersistedState(selfName: selfName, state: state),
    );
    return MinosCore._(client, secure);
  }

  /// Decide which [MobileClient] to hand back to callers at startup,
  /// recovering from a stale persisted snapshot when resume fails.
  ///
  /// The recovery branch matters because the Rust client retains the
  /// persisted device id even when the bearer is no longer valid: a
  /// subsequent reconnect would otherwise re-use that identity against an
  /// authenticated row on the backend and be rejected with 401. Dropping
  /// the snapshot lets the next pair attempt mint a fresh device.
  ///
  /// Phase 8.9: WS startup is now gated on the persisted auth tuple. If
  /// the snapshot has a device id but no `accessToken`, we hand back the
  /// rehydrated client *without* calling `resumePersistedSession` — the
  /// AuthController's stream listener will trigger the WS resume after
  /// the user logs in (`AuthAuthenticated`).
  ///
  /// Auth-only snapshots are valid too: login/register happens before QR
  /// pairing, so cold launch must keep the bearer tuple and stable device id.
  @visibleForTesting
  static Future<MobileClient> resolveClient({
    required SecurePairingStore secure,
    required MobileClient Function() buildFresh,
    required MobileClient Function(PersistedPairingState) buildFromPersisted,
  }) async {
    final persisted = await secure.loadState();
    if (persisted == null) return buildFresh();

    final client = buildFromPersisted(persisted);
    if (_hasPersistedAuth(persisted)) {
      try {
        await client.refreshSession();
        await _saveClientStateBestEffort(secure, client);
      } catch (_) {
        // The refresh token is the server-side proof that this cached login
        // is still usable. If validation fails, drop only auth so the user is
        // sent back to login while any pairing credential can be reused later.
        await secure.clearAuth();
        return client;
      }
    }

    if (persisted.accessToken == null) {
      // Paired-but-logged-out. Don't attempt the WS yet; AuthController
      // will retry resume after the next login.
      return client;
    }
    try {
      await client.resumePersistedSession();
      await _saveClientStateBestEffort(secure, client);
      return client;
    } catch (error) {
      if (_shouldDiscardPersistedState(error)) {
        await secure.clearAll();
        return buildFresh();
      }
      return client;
    }
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {
    await _client.pairWithQrJson(qrJson: qrJson);
    try {
      await _secure.saveState(await _client.persistedPairingState());
    } catch (error, stackTrace) {
      await _rollbackFailedPersistedPairSave();
      Error.throwWithStackTrace(error, stackTrace);
    }
  }

  @override
  Future<void> forgetHost(String hostDeviceId) async {
    await _client.forgetHost(hostDeviceId: hostDeviceId);
  }

  @override
  Future<List<HostSummaryDto>> listPairedHosts() => _client.listPairedHosts();

  @override
  Future<String?> activeHost() => _client.activeHost();

  @override
  Future<void> setActiveHost(String hostDeviceId) =>
      _client.setActiveHost(hostDeviceId: hostDeviceId);

  @override
  Future<bool> hasPersistedPairing() async {
    final state = await _secure.loadState();
    return state?.deviceId != null && state?.accessToken != null;
  }

  @override
  Future<String?> peerDisplayName() => _secure.loadPeerDisplayName();

  @override
  Future<void> setPeerDisplayName(String? name) =>
      _secure.savePeerDisplayName(name);

  @override
  Future<MyProfileResponse> myProfile() => _client.myProfile();

  @override
  Future<MyProfileResponse> setMinosId({required String minosId}) =>
      _client.setMinosId(minosId: minosId);

  @override
  Future<List<UserSummary>> searchUsers({required String minosId}) =>
      _client.searchUsers(minosId: minosId);

  @override
  Future<FriendRequestSummary> createFriendRequest({
    required String targetMinosId,
  }) => _client.createFriendRequest(targetMinosId: targetMinosId);

  @override
  Future<FriendRequestsResponse> friendRequests() => _client.friendRequests();

  @override
  Future<FriendRequestSummary> acceptFriendRequest({
    required String requestId,
  }) => _client.acceptFriendRequest(requestId: requestId);

  @override
  Future<FriendRequestSummary> rejectFriendRequest({
    required String requestId,
  }) => _client.rejectFriendRequest(requestId: requestId);

  @override
  Future<FriendsResponse> friends() => _client.friends();

  @override
  Future<AgentSummary> registerAgent({
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
  }) => _client.registerAgent(
    name: name,
    description: description,
    runtimeAgent: runtimeAgent,
    model: model,
  );

  @override
  Future<ListAgentsResponse> listAgents() => _client.listAgents();

  @override
  Future<ConversationsResponse> conversations() => _client.conversations();

  @override
  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  }) => _client.ensureDirectConversation(friendAccountId: friendAccountId);

  @override
  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  }) => _client.createGroupConversation(
    title: title,
    memberAccountIds: memberAccountIds,
  );

  @override
  Future<void> addGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) => _client.addGroupMember(
    conversationId: conversationId,
    memberAccountId: memberAccountId,
  );

  @override
  Future<ConversationMembersResponse> conversationMembers({
    required String conversationId,
  }) => _client.conversationMembers(conversationId: conversationId);

  @override
  Future<ConversationAgentMembersResponse> listConversationAgents({
    required String conversationId,
  }) => _client.listConversationAgents(conversationId: conversationId);

  @override
  Future<void> addAgentToConversation({
    required String conversationId,
    required String agentId,
  }) => _client.addAgentToConversation(
    conversationId: conversationId,
    agentId: agentId,
  );

  @override
  Future<void> removeAgentFromConversation({
    required String conversationId,
    required String agentId,
  }) => _client.removeAgentFromConversation(
    conversationId: conversationId,
    agentId: agentId,
  );

  @override
  Future<ConversationReadResponse> markConversationRead({
    required String conversationId,
  }) => _client.markConversationRead(conversationId: conversationId);

  @override
  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int? beforeTsMs,
    int limit = 50,
  }) => _client.listChatMessages(
    conversationId: conversationId,
    beforeTsMs: beforeTsMs == null ? null : platformInt64FromInt(beforeTsMs),
    limit: limit,
  );

  @override
  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
  }) => _client.sendChatMessage(
    conversationId: conversationId,
    text: text,
    replyToMessageId: replyToMessageId,
  );

  @override
  Future<ChatMessageSummary> recallChatMessage({
    required String conversationId,
    required String messageId,
  }) => _client.recallChatMessage(
    conversationId: conversationId,
    messageId: messageId,
  );

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) =>
      _client.listThreads(req: params);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) =>
      _client.readThread(req: params);

  // ---- Project forwarders ----

  @override
  Future<CreateProjectResponse> createProject({
    required String name,
    required String workspaceSlug,
  }) => _client.createProject(
    req: CreateProjectRequest(name: name, workspaceSlug: workspaceSlug),
  );

  @override
  Future<ListProjectsResponse> listProjects() => _client.listProjects();

  @override
  Future<void> updateProject({
    required String projectId,
    required String name,
  }) => _client.updateProject(
    req: UpdateProjectRequest(projectId: projectId, name: name),
  );

  @override
  Future<void> deleteProject({required String projectId}) =>
      _client.deleteProject(req: DeleteProjectRequest(projectId: projectId));

  @override
  Future<ListProjectThreadsResponse> listProjectThreads({
    required String projectId,
    int limit = 50,
    int? beforeTsMs,
  }) => _client.listProjectThreads(
    req: ListProjectThreadsParams(
      projectId: projectId,
      limit: limit,
      beforeTsMs: beforeTsMs == null ? null : platformInt64FromInt(beforeTsMs),
    ),
  );

  @override
  Stream<ConnectionState> get connectionStates => _client.subscribeState();

  @override
  Stream<UiEventFrame> get uiEvents => _client.subscribeUiEvents();

  @override
  Stream<SocialEventFrame> get socialEvents => _client.subscribeSocialEvents();

  @override
  ConnectionState get currentConnectionState => _client.currentState();

  // ---- Auth forwarders ----

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async {
    final summary = await _client.register(email: email, password: password);
    await _onAuthLanded(summary.accountId);
    return summary;
  }

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async {
    final summary = await _client.login(email: email, password: password);
    await _onAuthLanded(summary.accountId);
    return summary;
  }

  @override
  Future<void> refreshSession() async {
    await _client.refreshSession();
    await _saveClientStateBestEffort(_secure, _client);
  }

  @override
  Future<void> logout() async {
    await _client.logout();
    // Mirror the Rust-side wipe into the Dart keychain so a cold relaunch
    // doesn't rehydrate the dead session. The persisted device id is left
    // intact so the next account login on this device reuses it.
    await _secure.clearAuth();
  }

  /// Post-auth persistence (Phase 11.3 + ADR-0020).
  ///
  /// After a successful `register` / `login` we mirror the freshly minted
  /// auth tuple from the Rust core into the Dart keychain so a cold
  /// relaunch can rehydrate `auth_session` synchronously and the
  /// AuthController's first frame is already `Authenticated`.
  ///
  /// Cross-account migration: post ADR-0020 the pairing is account-scoped
  /// on the server (`account_mac_pairings`). Logging in as a different
  /// account simply yields a different `listPairedHosts` result on the
  /// next WS upgrade — no local "forget" call is needed. The peer display
  /// name from the previous account is cleared so a stale label doesn't
  /// show up before the first Mac sync.
  ///
  /// Best-effort throughout: the Rust side is the source of truth for
  /// the live session, so a keychain write failure does not invalidate
  /// the in-memory login. The next pair-or-resume cycle will recover.
  Future<void> _onAuthLanded(String newAccountId) async {
    final prior = await _secure.loadState();
    final priorAccountId = prior?.accountId;
    if (priorAccountId != null && priorAccountId != newAccountId) {
      // Different account: clear the peer display name from the previous
      // account so the partners list re-fetches from the server.
      try {
        await _secure.savePeerDisplayName(null);
      } catch (_) {
        // Best effort: a keychain write failure here is harmless — the
        // next listPairedHosts sync will overwrite the cached label.
      }
    }
    try {
      await _secure.saveState(await _client.persistedPairingState());
    } catch (_) {
      // Same rationale as above — the in-memory session is the source
      // of truth; persistence is a cold-launch optimisation.
    }
  }

  // ---- Agent dispatch forwarders ----

  @override
  Future<List<AgentDescriptor>> listClis() => _client.listClis();

  @override
  Future<ListHostSkillsResponse> listHostSkills({
    String? hostDeviceId,
    bool forceReload = true,
  }) => _client.listHostSkills(
    hostDeviceId: hostDeviceId,
    forceReload: forceReload,
  );

  @override
  Future<WriteHostSkillConfigResponse> writeHostSkillConfig({
    String? hostDeviceId,
    required String path,
    required bool enabled,
  }) => _client.writeHostSkillConfig(
    hostDeviceId: hostDeviceId,
    path: path,
    enabled: enabled,
  );

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) => _client.sendUserMessage(sessionId: sessionId, text: text);

  @override
  Future<void> interruptThread({required String threadId}) =>
      _client.interruptThread(threadId: threadId);

  @override
  Future<void> closeThread({required String threadId}) =>
      _client.closeThread(threadId: threadId);

  @override
  Future<void> deleteThread({required String threadId}) =>
      _client.closeThread(threadId: threadId);

  // ---- Lifecycle forwarders ----

  @override
  void notifyForegrounded() => _client.notifyForegrounded();

  @override
  void notifyBackgrounded() => _client.notifyBackgrounded();

  @override
  Stream<AuthStateFrame> get authStates async* {
    await for (final frame in _client.subscribeAuthState()) {
      if (frame is AuthStateFrame_Authenticated) {
        await _saveClientStateBestEffort(_secure, _client);
      } else if (frame is AuthStateFrame_Unauthenticated ||
          frame is AuthStateFrame_RefreshFailed) {
        try {
          await _secure.clearAuth();
        } catch (_) {
          // Keep auth-state delivery best-effort even if keychain writes fail.
        }
      }
      yield frame;
    }
  }

  @override
  Future<void> resumePersistedSession() async {
    await _client.resumePersistedSession();
    await _saveClientStateBestEffort(_secure, _client);
  }

  @override
  Future<void> sendApprovalDecision({
    required String requestId,
    required String threadId,
    required Map<String, dynamic> decision,
  }) => _client.sendApprovalDecision(
    requestId: requestId,
    threadId: threadId,
    decisionJson: jsonEncode(decision),
  );

  Future<void> _rollbackFailedPersistedPairSave() async {
    // ADR-0020: with bearer-only auth the server's `account_host_pairings`
    // row is the source of truth for the pairing — we can't atomically
    // un-pair without the just-minted host_device_id, which the failed
    // persistedPairingState() may not have surfaced. Best-effort: drop
    // any partial keychain snapshot so the next launch starts clean. The
    // user can forget the Mac explicitly from the Partners tab if the
    // server-side pairing turns out to be stale.
    try {
      await _secure.clearAll();
    } catch (_) {
      // Preserve the original persistence failure; the next launch will still
      // treat any leftover partial snapshot as non-resumable.
    }
  }

  static bool _shouldDiscardPersistedState(Object error) {
    return error is MinosError_DeviceNotTrusted ||
        error is MinosError_Unauthorized ||
        error is MinosError_StoreCorrupt;
  }

  static bool _hasPersistedAuth(PersistedPairingState state) {
    return state.accessToken != null &&
        state.accessExpiresAtMs != null &&
        state.refreshToken != null &&
        state.accountId != null &&
        state.accountEmail != null;
  }

  static Future<void> _saveClientStateBestEffort(
    SecurePairingStore secure,
    MobileClient client,
  ) async {
    try {
      await secure.saveState(await client.persistedPairingState());
    } catch (_) {
      // Persistence is a cold-launch optimisation. The live Rust session is
      // authoritative for the current process; a later login/pair/refresh can
      // repair the durable snapshot.
    }
  }
}
