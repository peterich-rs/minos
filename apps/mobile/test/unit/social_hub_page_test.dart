import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/social_hub_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  const _FakeCore({required this.friendsResponse});

  final FriendsResponse friendsResponse;

  @override
  Future<FriendRequestsResponse> friendRequests() async {
    return const FriendRequestsResponse(
      incoming: <FriendRequestSummary>[],
      outgoing: <FriendRequestSummary>[],
    );
  }

  @override
  Future<FriendsResponse> friends() async => friendsResponse;

  @override
  Future<MyProfileResponse> myProfile() async {
    return const MyProfileResponse(
      accountId: 'acct-me',
      email: 'me@example.com',
      minosId: 'me',
      displayName: 'Me',
    );
  }

  @override
  Future<List<UserSummary>> searchUsers({required String minosId}) async {
    return const <UserSummary>[];
  }

  @override
  dynamic noSuchMethod(Invocation invocation) {
    throw UnimplementedError('Unexpected call: $invocation');
  }
}

void main() {
  testWidgets('SocialHubPage shows a visible group creation entry', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        minosCoreProvider.overrideWithValue(
          const _FakeCore(
            friendsResponse: FriendsResponse(
              friends: <FriendSummary>[
                FriendSummary(
                  accountId: 'acct-1',
                  minosId: 'alice',
                  displayName: 'Alice',
                  createdAtMs: 1,
                ),
                FriendSummary(
                  accountId: 'acct-2',
                  minosId: 'bob',
                  displayName: 'Bob',
                  createdAtMs: 2,
                ),
              ],
            ),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const ShadApp(home: SocialHubPage()),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('新建群聊'), findsOneWidget);
    expect(find.text('从 2 位好友中选择成员'), findsOneWidget);
  });
}
