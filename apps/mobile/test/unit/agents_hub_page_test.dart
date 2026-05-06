import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/infrastructure/agent_profile_store.dart';
import 'package:minos/presentation/pages/agents_hub_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  const _FakeCore({
    this.skillsResponse = const ListHostSkillsResponse(
      data: <HostSkillsEntry>[],
    ),
  });

  final ListHostSkillsResponse skillsResponse;

  @override
  Future<String?> activeHost() async => null;

  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Future<void> closeThread({required String threadId}) async {}

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();

  @override
  Future<void> forgetHost(String hostDeviceId) async {}

  @override
  Future<FriendRequestSummary> acceptFriendRequest({
    required String requestId,
  }) async => throw UnimplementedError();

  @override
  Future<List<AgentDescriptor>> listClis() async => const <AgentDescriptor>[];

  @override
  Future<ListHostSkillsResponse> listHostSkills({
    String? hostDeviceId,
    bool forceReload = true,
  }) async => skillsResponse;

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async =>
      const ListThreadsResponse(threads: <ThreadSummary>[]);

  @override
  Future<ConversationsResponse> conversations() async =>
      const ConversationsResponse(conversations: <ConversationSummary>[]);

  @override
  Future<FriendRequestSummary> createFriendRequest({
    required String targetMinosId,
  }) async => throw UnimplementedError();

  @override
  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  }) async => throw UnimplementedError();

  @override
  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  }) async => throw UnimplementedError();

  @override
  Future<FriendRequestsResponse> friendRequests() async =>
      const FriendRequestsResponse(
        incoming: <FriendRequestSummary>[],
        outgoing: <FriendRequestSummary>[],
      );

  @override
  Future<FriendsResponse> friends() async =>
      const FriendsResponse(friends: <FriendSummary>[]);

  @override
  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int? beforeTsMs,
    int limit = 50,
  }) async => const ListChatMessagesResponse(messages: <ChatMessageSummary>[]);

  @override
  Future<MyProfileResponse> myProfile() async => const MyProfileResponse(
    accountId: 'acc',
    email: 'test@example.com',
    minosId: 'Test001',
  );

  @override
  Future<FriendRequestSummary> rejectFriendRequest({
    required String requestId,
  }) async => throw UnimplementedError();

  @override
  Future<List<UserSummary>> searchUsers({required String minosId}) async =>
      const <UserSummary>[];

  @override
  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
  }) async => throw UnimplementedError();

  @override
  Future<MyProfileResponse> setMinosId({required String minosId}) async =>
      MyProfileResponse(
        accountId: 'acc',
        email: 'test@example.com',
        minosId: minosId,
      );

  @override
  Future<List<HostSummaryDto>> listPairedHosts() async =>
      const <HostSummaryDto>[];

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<void> logout() async {}

  @override
  void notifyBackgrounded() {}

  @override
  void notifyForegrounded() {}

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<String?> peerDisplayName() async => null;

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async =>
      const ReadThreadResponse(uiEvents: <UiEventMessage>[]);

  @override
  Future<void> refreshSession() async {}

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<void> resumePersistedSession() async {}

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {}

  @override
  Future<WriteHostSkillConfigResponse> writeHostSkillConfig({
    String? hostDeviceId,
    required String path,
    required bool enabled,
  }) async => WriteHostSkillConfigResponse(effectiveEnabled: enabled);

  @override
  Future<void> setActiveHost(String hostDeviceId) async {}

  @override
  Future<void> setPeerDisplayName(String? name) async {}

  @override
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  }) async => const StartAgentResponse(sessionId: 'thr-1', cwd: '/tmp');

  @override
  Stream<UiEventFrame> get uiEvents => const Stream<UiEventFrame>.empty();
}

class _MemoryStore implements AgentProfileStore {
  @override
  Future<AgentWorkspaceState> load() async => AgentWorkspaceState.bootstrap();

  @override
  Future<void> save(AgentWorkspaceState state) async {}
}

void main() {
  testWidgets('AgentsHubTab renders the agent shell', (tester) async {
    final container = ProviderContainer(
      overrides: [
        minosCoreProvider.overrideWithValue(
          const _FakeCore(
            skillsResponse: ListHostSkillsResponse(
              data: <HostSkillsEntry>[
                HostSkillsEntry(
                  cwd: '/workspace',
                  errors: <HostSkillError>[],
                  skills: <HostSkillSummary>[
                    HostSkillSummary(
                      name: 'agent-review',
                      path: '/workspace/.agents/review/SKILL.md',
                      description: 'Review code changes.',
                      enabled: true,
                      scope: 'repo',
                      displayName: 'Review',
                      shortDescription: 'Checks diffs for regressions.',
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
        agentProfileStoreProvider.overrideWithValue(_MemoryStore()),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: AgentsHubTab()),
      ),
    );
    await tester.pump();

    expect(find.text('Agent'), findsOneWidget);
    expect(find.text('Profiles'), findsOneWidget);
  });
}
