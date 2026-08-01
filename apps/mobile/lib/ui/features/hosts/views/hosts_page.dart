import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_empty_state.dart';
import 'package:minos/ui/core/widgets/minos_page_header.dart';
import 'package:minos/ui/core/widgets/minos_surface.dart';
import 'package:minos/ui/core/widgets/shimmer_box.dart';
import 'package:minos/ui/features/hosts/widgets/host_card.dart';
import 'package:minos/ui/theme/theme.dart';

/// Golden-path Hosts tab: linked Macs from `GET /v1/hosts`.
class HostsPage extends ConsumerWidget {
  const HostsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final hostsAsync = ref.watch(pairedMacsProvider);
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final colors = context.minosColors;

    return ColoredBox(
      color: colors.canvas,
      child: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            MinosPageHeader(
              title: 'Hosts',
              subtitle: _connectionSubtitle(connection),
              trailing: IconButton(
                tooltip: '刷新',
                onPressed: () => _refresh(context, ref),
                icon: const Icon(CupertinoIcons.arrow_clockwise),
              ),
            ),
            Expanded(
              child: RefreshIndicator(
                onRefresh: () => _refresh(context, ref),
                child: hostsAsync.when(
                  loading: () => const _HostsSkeleton(),
                  error: (error, _) => ListView(
                    physics: const AlwaysScrollableScrollPhysics(
                      parent: BouncingScrollPhysics(),
                    ),
                    children: <Widget>[
                      Padding(
                        padding: const EdgeInsets.all(MinosSpacing.pageX),
                        child: MinosEmptyState(
                          icon: CupertinoIcons.exclamationmark_triangle,
                          title: 'Hosts 暂时不可用',
                          subtitle: error.toString(),
                          actionLabel: '重试',
                          onAction: () => _refresh(context, ref),
                        ),
                      ),
                    ],
                  ),
                  data: (hosts) {
                    if (hosts.isEmpty) {
                      return ListView(
                        physics: const AlwaysScrollableScrollPhysics(
                          parent: BouncingScrollPhysics(),
                        ),
                        children: const <Widget>[
                          MinosEmptyState(
                            icon: CupertinoIcons.desktopcomputer,
                            title: '还没有 Linked Host',
                            subtitle:
                                '在 Desktop 用同一 Minos 账号打开应用，点击 Link this Mac。完成后下拉刷新即可看到。',
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
                      itemCount: hosts.length,
                      separatorBuilder: (_, _) =>
                          const SizedBox(height: MinosSpacing.md),
                      itemBuilder: (context, index) {
                        final host = hosts[index];
                        return HostCard(
                          displayName: host.hostDisplayName,
                          online: host.online,
                          selected: host.hostDeviceId == activeHostId,
                          onTap: () => ref
                              .read(activeMacProvider.notifier)
                              .setActive(host.hostDeviceId),
                          subtitle: host.online
                              ? (host.hostDeviceId == activeHostId
                                    ? '在线 · 当前路由目标'
                                    : '在线 · 点按设为路由目标')
                              : '离线 · Desktop 需保持 Link 并连接',
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
      await ref.read(pairedMacsProvider.notifier).refresh();
      await ref.read(activeMacProvider.notifier).refresh();
    } catch (error) {
      if (context.mounted) {
        showLoggedErrorToast(
          context,
          target: 'hosts_page',
          title: 'Hosts 刷新失败',
          error: error,
        );
      }
    }
  }

  static String _connectionSubtitle(ConnectionState? state) {
    return switch (state) {
      ConnectionState_Connected() => '实时通道已连接',
      ConnectionState_Reconnecting(:final attempt) => '重连中 #$attempt',
      ConnectionState_Pairing() => '建立实时连接…',
      _ => '实时通道离线',
    };
  }
}

class _HostsSkeleton extends StatelessWidget {
  const _HostsSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(
        MinosSpacing.pageX,
        MinosSpacing.sm,
        MinosSpacing.pageX,
        MinosSpacing.pageBottom,
      ),
      children: List.generate(3, (index) {
        return Padding(
          padding: EdgeInsets.only(bottom: index < 2 ? MinosSpacing.md : 0),
          child: const MinosSurface(
            bordered: true,
            padding: EdgeInsets.all(MinosSpacing.lg),
            child: Row(
              children: <Widget>[
                ShimmerBox(width: 48, height: 48, borderRadius: MinosRadii.sm),
                SizedBox(width: MinosSpacing.md),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      ShimmerBox(width: 140, height: 14),
                      SizedBox(height: MinosSpacing.sm),
                      ShimmerBox(width: 100, height: 12),
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
