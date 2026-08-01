import 'dart:async';

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_empty_state.dart';
import 'package:minos/ui/core/widgets/minos_page_header.dart';
import 'package:minos/ui/core/widgets/minos_surface.dart';
import 'package:minos/ui/core/widgets/shimmer_box.dart';
import 'package:minos/ui/features/sessions/widgets/session_tile.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/theme/theme.dart';

/// Golden-path Sessions inbox: flat list of agent sessions.
class SessionsPage extends ConsumerWidget {
  const SessionsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sessionsAsync = ref.watch(threadListProvider);
    final hosts = ref.watch(pairedMacsProvider).asData?.value ?? const [];
    final hasHosts = hosts.isNotEmpty;
    final colors = context.minosColors;

    return ColoredBox(
      color: colors.canvas,
      child: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            MinosPageHeader(
              title: 'Sessions',
              subtitle: hasHosts
                  ? '从 Linked Host 同步的对话'
                  : '先在 Hosts 查看 Linked Mac',
              trailing: IconButton(
                tooltip: '新建对话',
                onPressed: () {
                  ref.read(activeSessionControllerProvider.notifier).reset();
                  unawaited(context.push(AppRoutes.agentStart));
                },
                icon: Icon(CupertinoIcons.square_pencil, color: colors.accent),
              ),
            ),
            Expanded(
              child: RefreshIndicator(
                onRefresh: () => _refresh(context, ref),
                child: sessionsAsync.when(
                  loading: () => const _SessionsSkeleton(),
                  error: (error, _) => ListView(
                    physics: const AlwaysScrollableScrollPhysics(
                      parent: BouncingScrollPhysics(),
                    ),
                    children: <Widget>[
                      MinosEmptyState(
                        icon: CupertinoIcons.exclamationmark_triangle,
                        title: '会话列表不可用',
                        subtitle: error.toString(),
                        actionLabel: '重试',
                        onAction: () => _refresh(context, ref),
                      ),
                    ],
                  ),
                  data: (sessions) {
                    if (sessions.isEmpty) {
                      return ListView(
                        physics: const AlwaysScrollableScrollPhysics(
                          parent: BouncingScrollPhysics(),
                        ),
                        children: <Widget>[
                          MinosEmptyState(
                            icon: CupertinoIcons.chat_bubble_2,
                            title: hasHosts ? '还没有会话' : '还没有 Linked Host',
                            subtitle: hasHosts
                                ? '点右上角新建对话，或在 Desktop 开始一个 Agent 会话。'
                                : '在 Desktop 用同一账号 Link this Mac，然后下拉刷新。',
                            actionLabel: hasHosts ? '新建对话' : null,
                            onAction: hasHosts
                                ? () {
                                    ref
                                        .read(
                                          activeSessionControllerProvider
                                              .notifier,
                                        )
                                        .reset();
                                    unawaited(
                                      context.push(AppRoutes.agentStart),
                                    );
                                  }
                                : null,
                          ),
                        ],
                      );
                    }

                    return ListView.separated(
                      physics: const AlwaysScrollableScrollPhysics(
                        parent: BouncingScrollPhysics(),
                      ),
                      padding: const EdgeInsets.fromLTRB(
                        MinosSpacing.pageX,
                        MinosSpacing.sm,
                        MinosSpacing.pageX,
                        MinosSpacing.pageBottom,
                      ),
                      itemCount: sessions.length,
                      separatorBuilder: (_, _) =>
                          const SizedBox(height: MinosSpacing.sm),
                      itemBuilder: (context, index) {
                        final session = sessions[index];
                        return SessionTile(
                          session: session,
                          onTap: () {
                            unawaited(
                              context.push(
                                '/thread/${session.sessionId}',
                                extra: ThreadRouteExtra(agent: session.agent),
                              ),
                            );
                          },
                        );
                      },
                    );
                  },
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _refresh(BuildContext context, WidgetRef ref) async {
    try {
      await ref.read(threadListProvider.notifier).refresh();
      await ref.read(pairedMacsProvider.notifier).refresh();
    } catch (error) {
      if (context.mounted) {
        showLoggedErrorToast(
          context,
          target: 'sessions_page',
          title: '会话刷新失败',
          error: error,
        );
      }
    }
  }
}

class _SessionsSkeleton extends StatelessWidget {
  const _SessionsSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(
        MinosSpacing.pageX,
        MinosSpacing.sm,
        MinosSpacing.pageX,
        MinosSpacing.pageBottom,
      ),
      children: List.generate(6, (index) {
        return const Padding(
          padding: EdgeInsets.only(bottom: MinosSpacing.sm),
          child: MinosSurface(
            bordered: true,
            padding: EdgeInsets.all(MinosSpacing.lg),
            child: Row(
              children: <Widget>[
                ShimmerBox(width: 42, height: 42, borderRadius: MinosRadii.sm),
                SizedBox(width: MinosSpacing.md),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      ShimmerBox(width: 160, height: 13),
                      SizedBox(height: MinosSpacing.sm),
                      ShimmerBox(width: 110, height: 11),
                    ],
                  ),
                ),
              ],
            ),
          ),
        );
      }),
    );
  }
}
