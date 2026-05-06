import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/social_providers.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/presentation/pages/social_chat_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class SocialHubPage extends ConsumerStatefulWidget {
  const SocialHubPage({super.key});

  @override
  ConsumerState<SocialHubPage> createState() => _SocialHubPageState();
}

class _SocialHubPageState extends ConsumerState<SocialHubPage> {
  final TextEditingController _searchController = TextEditingController();
  String _query = '';

  @override
  void dispose() {
    _searchController.dispose();
    super.dispose();
  }

  Future<void> _refreshAll() async {
    await Future.wait<void>(<Future<void>>[
      ref.read(friendRequestsProvider.notifier).refresh(),
      ref.read(friendsProvider.notifier).refresh(),
      ref.read(conversationsProvider.notifier).refresh(),
    ]);
    ref.invalidate(socialProfileProvider);
  }

  Future<void> _openDirectChat(FriendSummary friend) async {
    try {
      final response = await ref
          .read(minosCoreProvider)
          .ensureDirectConversation(friendAccountId: friend.accountId);
      if (!mounted) return;
      ref.invalidate(conversationsProvider);
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => SocialChatPage(
            conversationId: response.conversationId,
            title: friend.displayName,
            kind: ConversationKind.direct,
          ),
        ),
      );
    } catch (error) {
      if (!mounted) return;
      _showSocialError(context, '打开聊天失败', error);
    }
  }

  Future<void> _editMinosId(MyProfileResponse profile) async {
    final controller = TextEditingController(text: profile.minosId);
    final rootContext = context;
    await showShadDialog(
      context: rootContext,
      builder: (context) => ShadDialog.alert(
        title: const Text('设置 Minos ID'),
        description: const Text('仅允许数字和英文字母，区分大小写。'),
        actions: [
          ShadButton.outline(
            child: const Text('取消'),
            onPressed: () => Navigator.of(context).pop(),
          ),
          ShadButton(
            child: const Text('保存'),
            onPressed: () async {
              try {
                await ref
                    .read(minosCoreProvider)
                    .setMinosId(minosId: controller.text.trim());
                ref.invalidate(socialProfileProvider);
                if (context.mounted) Navigator.of(context).pop();
              } catch (error) {
                if (!mounted || !rootContext.mounted) return;
                _showSocialError(rootContext, '设置失败', error);
              }
            },
          ),
        ],
        child: Padding(
          padding: const EdgeInsets.only(top: 12),
          child: ShadInput(controller: controller),
        ),
      ),
    );
  }

  Future<void> _createGroup(List<FriendSummary> friends) async {
    final titleController = TextEditingController();
    final selectedIds = <String>{};
    final rootContext = context;
    await showModalBottomSheet<void>(
      context: rootContext,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (context) {
        return StatefulBuilder(
          builder: (context, setSheetState) {
            return Padding(
              padding: EdgeInsets.only(
                left: 16,
                right: 16,
                top: 16,
                bottom: MediaQuery.of(context).viewInsets.bottom + 16,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text('新建群聊', style: ShadTheme.of(context).textTheme.h4),
                  const SizedBox(height: 12),
                  ShadInput(
                    controller: titleController,
                    placeholder: const Text('群聊名称'),
                  ),
                  const SizedBox(height: 12),
                  Flexible(
                    child: ListView(
                      shrinkWrap: true,
                      children: [
                        for (final friend in friends)
                          CheckboxListTile(
                            value: selectedIds.contains(friend.accountId),
                            onChanged: (selected) {
                              setSheetState(() {
                                if (selected == true) {
                                  selectedIds.add(friend.accountId);
                                } else {
                                  selectedIds.remove(friend.accountId);
                                }
                              });
                            },
                            title: Text(friend.displayName),
                            subtitle: Text(friend.minosId),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 12),
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: ShadButton.outline(
                          child: const Text('取消'),
                          onPressed: () => Navigator.of(context).pop(),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: ShadButton(
                          child: const Text('创建'),
                          onPressed: () async {
                            try {
                              final response = await ref
                                  .read(minosCoreProvider)
                                  .createGroupConversation(
                                    title: titleController.text.trim(),
                                    memberAccountIds: selectedIds.toList(),
                                  );
                              if (!context.mounted) return;
                              ref.invalidate(conversationsProvider);
                              Navigator.of(context).pop();
                              if (!mounted) return;
                              await Navigator.of(rootContext).push(
                                MaterialPageRoute<void>(
                                  builder: (_) => SocialChatPage(
                                    conversationId: response.conversationId,
                                    title: titleController.text.trim().isEmpty
                                        ? '群聊'
                                        : titleController.text.trim(),
                                    kind: ConversationKind.group,
                                  ),
                                ),
                              );
                            } catch (error) {
                              if (!mounted || !rootContext.mounted) return;
                              _showSocialError(rootContext, '创建群聊失败', error);
                            }
                          },
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            );
          },
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final profileAsync = ref.watch(socialProfileProvider);
    final requestsAsync = ref.watch(friendRequestsProvider);
    final friendsAsync = ref.watch(friendsProvider);
    final searchAsync = ref.watch(socialSearchProvider(_query));
    final shadTheme = ShadTheme.of(context);

    return Scaffold(
      backgroundColor: shadTheme.colorScheme.background,
      appBar: AppBar(
        title: const Text('People'),
        surfaceTintColor: Colors.transparent,
      ),
      body: RefreshIndicator(
        onRefresh: _refreshAll,
        child: ListView(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 28),
          children: <Widget>[
            profileAsync.when(
              loading: () => const SizedBox(
                height: 100,
                child: Center(child: ShadProgress()),
              ),
              error: (error, _) => _SocialSection(
                title: '我的 ID',
                child: _SectionMessage(text: error.toString()),
              ),
              data: (profile) => _SocialSection(
                title: '我的 ID',
                trailing: ShadButton.ghost(
                  onPressed: () => _editMinosId(profile),
                  child: const Text('编辑'),
                ),
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      Text(profile.email, style: shadTheme.textTheme.small),
                      const SizedBox(height: 8),
                      Text(profile.minosId, style: shadTheme.textTheme.h3),
                    ],
                  ),
                ),
              ),
            ),
            const SizedBox(height: 16),
            _SocialSection(
              title: '添加好友',
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  children: <Widget>[
                    ShadInput(
                      controller: _searchController,
                      placeholder: const Text('输入 Minos ID'),
                      onChanged: (value) =>
                          setState(() => _query = value.trim()),
                    ),
                    if (_query.isNotEmpty) ...<Widget>[
                      const SizedBox(height: 12),
                      searchAsync.when(
                        loading: () => const Center(child: ShadProgress()),
                        error: (error, _) =>
                            _SectionMessage(text: error.toString()),
                        data: (users) => Column(
                          children: [
                            for (final user in users)
                              ListTile(
                                contentPadding: EdgeInsets.zero,
                                title: Text(user.displayName),
                                subtitle: Text(user.minosId),
                                trailing: ShadButton.outline(
                                  onPressed: () async {
                                    try {
                                      await ref
                                          .read(minosCoreProvider)
                                          .createFriendRequest(
                                            targetMinosId: user.minosId,
                                          );
                                      await ref
                                          .read(friendRequestsProvider.notifier)
                                          .refresh();
                                      if (context.mounted) {
                                        _showSocialInfo(context, '请求已发送');
                                      }
                                    } catch (error) {
                                      if (!context.mounted) return;
                                      _showSocialError(
                                        context,
                                        '发送请求失败',
                                        error,
                                      );
                                    }
                                  },
                                  child: const Text('添加'),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ),
            const SizedBox(height: 16),
            requestsAsync.when(
              loading: () => _SocialSection(
                title: '好友请求',
                child: const Padding(
                  padding: EdgeInsets.all(16),
                  child: Center(child: ShadProgress()),
                ),
              ),
              error: (error, _) => _SocialSection(
                title: '好友请求',
                child: _SectionMessage(text: error.toString()),
              ),
              data: (requests) => _SocialSection(
                title: '好友请求',
                child: requests.incoming.isEmpty && requests.outgoing.isEmpty
                    ? const _SectionMessage(text: '暂无好友请求')
                    : Column(
                        children: [
                          for (final request in requests.incoming)
                            ListTile(
                              title: Text(request.from.displayName),
                              subtitle: Text(request.from.minosId),
                              trailing: Row(
                                mainAxisSize: MainAxisSize.min,
                                children: <Widget>[
                                  ShadButton.outline(
                                    onPressed: () async {
                                      try {
                                        await ref
                                            .read(minosCoreProvider)
                                            .rejectFriendRequest(
                                              requestId: request.requestId,
                                            );
                                        await _refreshAll();
                                        if (!context.mounted) return;
                                      } catch (error) {
                                        _showSocialError(
                                          context,
                                          '拒绝失败',
                                          error,
                                        );
                                      }
                                    },
                                    child: const Text('拒绝'),
                                  ),
                                  const SizedBox(width: 8),
                                  ShadButton(
                                    onPressed: () async {
                                      try {
                                        await ref
                                            .read(minosCoreProvider)
                                            .acceptFriendRequest(
                                              requestId: request.requestId,
                                            );
                                        await _refreshAll();
                                        if (!context.mounted) return;
                                      } catch (error) {
                                        _showSocialError(
                                          context,
                                          '接受失败',
                                          error,
                                        );
                                      }
                                    },
                                    child: const Text('接受'),
                                  ),
                                ],
                              ),
                            ),
                          for (final request in requests.outgoing)
                            ListTile(
                              title: Text(request.to.displayName),
                              subtitle: Text('${request.to.minosId} · 已发送'),
                            ),
                        ],
                      ),
              ),
            ),
            const SizedBox(height: 16),
            friendsAsync.when(
              loading: () => _SocialSection(
                title: '好友',
                child: const Padding(
                  padding: EdgeInsets.all(16),
                  child: Center(child: ShadProgress()),
                ),
              ),
              error: (error, _) => _SocialSection(
                title: '好友',
                child: _SectionMessage(text: error.toString()),
              ),
              data: (friends) => _SocialSection(
                title: '好友',
                trailing: friends.friends.length >= 2
                    ? ShadButton.ghost(
                        onPressed: () => _createGroup(friends.friends),
                        child: const Text('建群'),
                      )
                    : null,
                child: friends.friends.isEmpty
                    ? const _SectionMessage(text: '还没有好友')
                    : Column(
                        children: [
                          for (final friend in friends.friends)
                            ListTile(
                              title: Text(friend.displayName),
                              subtitle: Text(friend.minosId),
                              trailing: const Icon(LucideIcons.chevronRight),
                              onTap: () => _openDirectChat(friend),
                            ),
                        ],
                      ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SocialSection extends StatelessWidget {
  const _SocialSection({
    required this.title,
    required this.child,
    this.trailing,
  });

  final String title;
  final Widget child;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
          child: Row(
            children: <Widget>[
              Expanded(
                child: Text(
                  title,
                  style: shadTheme.textTheme.small.copyWith(
                    color: shadTheme.colorScheme.mutedForeground,
                  ),
                ),
              ),
              ...?trailing == null ? null : <Widget>[trailing!],
            ],
          ),
        ),
        DecoratedBox(
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surfaceContainerLow,
            borderRadius: BorderRadius.circular(16),
          ),
          child: child,
        ),
      ],
    );
  }
}

class _SectionMessage extends StatelessWidget {
  const _SectionMessage({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(padding: const EdgeInsets.all(16), child: Text(text));
  }
}

void _showSocialError(BuildContext context, String title, Object error) {
  ShadToaster.maybeOf(context)?.show(
    ShadToast.destructive(
      title: Text(title),
      description: Text(error.toString()),
    ),
  );
}

void _showSocialInfo(BuildContext context, String title) {
  ShadToaster.maybeOf(context)?.show(ShadToast(title: Text(title)));
}
