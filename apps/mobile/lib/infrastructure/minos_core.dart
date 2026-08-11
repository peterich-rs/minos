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
  /// WS startup is gated on the persisted auth tuple. If the snapshot has
  /// a device id but no `accessToken`, we hand back the rehydrated client
  /// *without* calling `resumePersistedSession` — the AuthController's
  /// stream listener will trigger the WS resume after the user logs in
  /// (`AuthAuthenticated`).
  ///
  /// Auth-only snapshots are valid too: exchange happens before QR
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
  Future<MyProfileResponse> myProfile() => _client.myProfile();

  @override
  Future<FriendsResponse> friends() => _client.friends();

  @override
  Future<AgentSummary> registerAgent({
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
    String? workspacePath,
    String? displayName,
    String? defaultReasoningEffort,
    String? systemPrompt,
  }) => _client.registerAgent(
    name: name,
    description: description,
    runtimeAgent: runtimeAgent,
    model: model,
    workspacePath: workspacePath,
    displayName: displayName,
    defaultReasoningEffort: defaultReasoningEffort,
    systemPrompt: systemPrompt,
  );

  @override
  Future<AgentSummary> updateAgent({
    required String agentId,
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
    String? workspacePath,
    String? displayName,
    String? defaultReasoningEffort,
    String? systemPrompt,
    String? status,
  }) => _client.updateAgent(
    agentId: agentId,
    name: name,
    description: description,
    runtimeAgent: runtimeAgent,
    model: model,
    workspacePath: workspacePath,
    displayName: displayName,
    defaultReasoningEffort: defaultReasoningEffort,
    systemPrompt: systemPrompt,
    status: status,
  );

  @override
  Future<ListAgentsResponse> listAgents() => _client.listAgents();

  @override
  Future<ConversationsResponse> conversations() => _client.conversations();

  @override
  Future<void> deleteConversation({required String conversationId}) =>
      _client.deleteConversation(conversationId: conversationId);

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
  Future<void> removeGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) => _client.removeGroupMember(
    conversationId: conversationId,
    memberAccountId: memberAccountId,
  );

  @override
  Future<ConversationParticipantsResponse> listConversationParticipants({
    required String conversationId,
  }) => _client.listConversationParticipants(conversationId: conversationId);

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
    required int readUpToMessageSeq,
  }) => _client.markConversationRead(
    conversationId: conversationId,
    readUpToMessageSeq: platformInt64FromInt(readUpToMessageSeq),
  );

  @override
  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int? beforeSeq,
    int? afterSeq,
    int limit = 50,
  }) => _client.listChatMessages(
    conversationId: conversationId,
    beforeSeq: beforeSeq == null ? null : platformInt64FromInt(beforeSeq),
    afterSeq: afterSeq == null ? null : platformInt64FromInt(afterSeq),
    limit: limit,
  );

  @override
  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
    String? clientMessageId,
    String? mentionsJson,
  }) => _client.sendChatMessage(
    conversationId: conversationId,
    text: text,
    replyToMessageId: replyToMessageId,
    clientMessageId: clientMessageId,
    mentionsJson: mentionsJson,
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
  Future<ToggleReactionResponse> toggleReaction({
    required String conversationId,
    required String messageId,
    required String emoji,
    required String clientOpId,
  }) => _client.toggleReaction(
    conversationId: conversationId,
    messageId: messageId,
    emoji: emoji,
    clientOpId: clientOpId,
  );

  @override
  Future<void> subscribeConversation({required String conversationId}) =>
      _client.subscribeConversation(conversationId: conversationId);

  @override
  Future<void> unsubscribeConversation({required String conversationId}) =>
      _client.unsubscribeConversation(conversationId: conversationId);

  @override
  Stream<ConnectionState> get connectionStates => _client.subscribeState();

  @override
  Stream<UiEventFrame> get uiEvents => _client.subscribeUiEvents();

  @override
  Stream<SocialEventFrame> get socialEvents => _client.subscribeSocialEvents();

  @override
  ConnectionState get currentConnectionState => _client.currentState();

  @override
  Future<void> ackDurableApplied({
    required String topic,
    required int topicSeq,
  }) => _client.ackDurableApplied(
    topic: topic,
    topicSeq: platformInt64FromInt(topicSeq),
  );

  // ---- Auth forwarders ----

  @override
  Future<AuthSummary> loginWithSupabase({
    required String supabaseAccessToken,
  }) async {
    final summary = await _client.loginWithSupabase(
      supabaseAccessToken: supabaseAccessToken,
    );
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

  /// Post-auth persistence.
  ///
  /// After a successful Supabase exchange we mirror the freshly minted
  /// auth tuple from the Rust core into the Dart keychain so a cold
  /// relaunch can rehydrate `auth_session` synchronously and the
  /// AuthController's first frame is already `Authenticated`.
  ///
  /// Cross-account migration: pairing is account-scoped on the server
  /// (`account_mac_pairings`). Logging in as a different account simply
  /// yields a different `listPairedHosts` result on the next WS upgrade —
  /// no local "forget" call is needed. The peer display name from the
  /// previous account is cleared so a stale label doesn't show up before
  /// the first Mac sync.
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
  Future<ListHostWorkspacesResponse> listHostWorkspaces({
    String? hostDeviceId,
    String? root,
    int limit = 100,
  }) => _client.listHostWorkspaces(
    hostDeviceId: hostDeviceId,
    root: root,
    limit: limit,
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
