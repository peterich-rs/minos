import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/social_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

class SocialProfileSection extends ConsumerWidget {
  const SocialProfileSection({super.key});

  Future<void> _editMinosId(
    BuildContext context,
    WidgetRef ref,
    MyProfileResponse profile,
  ) async {
    final controller = TextEditingController(text: profile.minosId);
    final rootContext = context;
    await showShadDialog<void>(
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
                    .read(socialActionsProvider)
                    .setMinosId(minosId: controller.text.trim());
                ref.invalidate(socialProfileProvider);
                if (context.mounted) Navigator.of(context).pop();
              } catch (error) {
                if (!rootContext.mounted) return;
                showSocialFeedbackError(rootContext, '设置失败', error);
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
    controller.dispose();
  }

  Future<void> _copyText(
    BuildContext context,
    String text,
    String title,
  ) async {
    await Clipboard.setData(ClipboardData(text: text));
    if (context.mounted) {
      showSocialInfoToast(context, title);
    }
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profileAsync = ref.watch(socialProfileProvider);
    final shadTheme = ShadTheme.of(context);
    return profileAsync.when(
      loading: () => const SocialSectionCard(
        title: '账户 ID',
        child: Padding(
          padding: EdgeInsets.all(16),
          child: Center(child: ShadProgress()),
        ),
      ),
      error: (error, _) => SocialSectionCard(
        title: '账户 ID',
        child: SocialSectionMessage(text: error.toString()),
      ),
      data: (profile) {
        final minosId = profile.minosId.trim().isEmpty
            ? '未设置'
            : profile.minosId.trim();
        return SocialSectionCard(
          title: '账户 ID',
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              ShadButton.ghost(
                onPressed: profile.minosId.trim().isEmpty
                    ? null
                    : () => _copyText(
                        context,
                        'minos://add-friend/${profile.minosId}',
                        '邀请链接已复制',
                      ),
                child: const Text('邀请'),
              ),
              ShadButton.ghost(
                onPressed: () => _editMinosId(context, ref, profile),
                child: const Text('编辑'),
              ),
            ],
          ),
          child: InkWell(
            onTap: profile.minosId.trim().isEmpty
                ? null
                : () => _copyText(context, profile.minosId, 'Minos ID 已复制'),
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(profile.email, style: shadTheme.textTheme.small),
                  const SizedBox(height: 10),
                  _ProfileValueRow(label: 'Minos ID', value: minosId),
                  const SizedBox(height: 10),
                  _ProfileValueRow(label: '账户 ID', value: profile.accountId),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

class FriendSearchSection extends ConsumerStatefulWidget {
  const FriendSearchSection({super.key});

  @override
  ConsumerState<FriendSearchSection> createState() =>
      _FriendSearchSectionState();
}

class _FriendSearchSectionState extends ConsumerState<FriendSearchSection> {
  final TextEditingController _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final query = ref.watch(socialSearchQueryProvider);
    final searchAsync = ref.watch(socialSearchProvider(query));
    return SocialSectionCard(
      title: '添加好友',
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          children: <Widget>[
            ShadInput(
              controller: _controller,
              placeholder: const Text('输入 Minos ID 或邀请链接'),
              onChanged: (value) {
                ref
                    .read(socialSearchQueryProvider.notifier)
                    .update(normalizeFriendSearchInput(value));
              },
            ),
            if (query.isNotEmpty) ...<Widget>[
              const SizedBox(height: 12),
              searchAsync.when(
                loading: () => const Center(child: ShadProgress()),
                error: (error, _) =>
                    SocialSectionMessage(text: error.toString()),
                data: (users) => users.isEmpty
                    ? const SocialSectionMessage(text: '没有找到这个用户')
                    : Column(
                        children: <Widget>[
                          for (final user in users)
                            ListTile(
                              contentPadding: EdgeInsets.zero,
                              title: Text(user.displayName),
                              subtitle: Text('@${user.minosId}'),
                              trailing: ShadButton.outline(
                                onPressed: () async {
                                  try {
                                    await ref
                                        .read(socialActionsProvider)
                                        .createFriendRequest(
                                          targetMinosId: user.minosId,
                                        );
                                    await ref
                                        .read(friendRequestsProvider.notifier)
                                        .refresh();
                                    if (context.mounted) {
                                      showSocialInfoToast(context, '请求已发送');
                                    }
                                  } catch (error) {
                                    if (!context.mounted) return;
                                    showSocialFeedbackError(
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
    );
  }
}

class FriendRequestsSection extends ConsumerWidget {
  const FriendRequestsSection({super.key});

  Future<void> _refreshSocialLists(WidgetRef ref) async {
    await Future.wait<void>(<Future<void>>[
      ref.read(friendRequestsProvider.notifier).refresh(),
      ref.read(friendsProvider.notifier).refresh(),
    ]);
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final requestsAsync = ref.watch(friendRequestsProvider);
    return requestsAsync.when(
      loading: () => const SocialSectionCard(
        title: '好友请求',
        child: Padding(
          padding: EdgeInsets.all(16),
          child: Center(child: ShadProgress()),
        ),
      ),
      error: (error, _) => SocialSectionCard(
        title: '好友请求',
        child: SocialSectionMessage(text: error.toString()),
      ),
      data: (requests) {
        final count = requests.incoming.length + requests.outgoing.length;
        return SocialSectionCard(
          title: count == 0 ? '好友请求' : '好友请求 $count',
          child: requests.incoming.isEmpty && requests.outgoing.isEmpty
              ? const SocialSectionMessage(text: '暂无好友请求')
              : Column(
                  children: <Widget>[
                    for (var i = 0; i < requests.incoming.length; i++) ...[
                      if (i > 0) const Divider(height: 1),
                      ListTile(
                        title: Text(requests.incoming[i].from.displayName),
                        subtitle: Text('@${requests.incoming[i].from.minosId}'),
                        trailing: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: <Widget>[
                            ShadButton.outline(
                              onPressed: () async {
                                try {
                                  await ref
                                      .read(socialActionsProvider)
                                      .rejectFriendRequest(
                                        requestId:
                                            requests.incoming[i].requestId,
                                      );
                                  await _refreshSocialLists(ref);
                                } catch (error) {
                                  if (!context.mounted) return;
                                  showSocialFeedbackError(
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
                                      .read(socialActionsProvider)
                                      .acceptFriendRequest(
                                        requestId:
                                            requests.incoming[i].requestId,
                                      );
                                  await _refreshSocialLists(ref);
                                } catch (error) {
                                  if (!context.mounted) return;
                                  showSocialFeedbackError(
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
                    ],
                    for (var i = 0; i < requests.outgoing.length; i++) ...[
                      if (requests.incoming.isNotEmpty || i > 0)
                        const Divider(height: 1),
                      ListTile(
                        title: Text(requests.outgoing[i].to.displayName),
                        subtitle: Text(
                          '@${requests.outgoing[i].to.minosId} · 已发送',
                        ),
                      ),
                    ],
                  ],
                ),
        );
      },
    );
  }
}

class SocialSectionCard extends StatelessWidget {
  const SocialSectionCard({
    super.key,
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
              ?trailing,
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

class SocialSectionMessage extends StatelessWidget {
  const SocialSectionMessage({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(padding: const EdgeInsets.all(16), child: Text(text));
  }
}

class _ProfileValueRow extends StatelessWidget {
  const _ProfileValueRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Text(label, style: shadTheme.textTheme.muted),
        const SizedBox(height: 4),
        Text(
          value,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: shadTheme.textTheme.h4,
        ),
      ],
    );
  }
}

String normalizeFriendSearchInput(String value) {
  final trimmed = value.trim();
  if (trimmed.startsWith('minos://add-friend/')) {
    return trimmed.substring('minos://add-friend/'.length);
  }
  final uri = Uri.tryParse(trimmed);
  if (uri != null &&
      uri.pathSegments.length >= 2 &&
      uri.pathSegments[uri.pathSegments.length - 2] == 'add-friend') {
    return uri.pathSegments.last;
  }
  return trimmed;
}

void showSocialFeedbackError(BuildContext context, String title, Object error) {
  showLoggedErrorToast(
    context,
    target: 'social_hub',
    title: title,
    error: error,
  );
}

void showSocialInfoToast(BuildContext context, String title) {
  ShadToaster.maybeOf(context)?.show(ShadToast(title: Text(title)));
}
