import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/presentation/pages/account_settings_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/thread_list_tile.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Landing page once auth + pairing settle. Shows the paginated thread
/// list with:
///
///   - top "Mac offline" banner when the WS isn't connected (per spec
///     §9.5: input must be hard-disabled while the agent host is
///     unreachable);
///   - app-bar account-settings button (Phase 11 wires the real page);
///   - floating action button to start a brand-new chat.
class ThreadListPage extends ConsumerWidget {
  const ThreadListPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final threadsAsync = ref.watch(threadListProvider);
    final connectionAsync = ref.watch(connectionStateProvider);
    final isConnected =
        connectionAsync.asData?.value is ConnectionState_Connected;

    return Scaffold(
      appBar: AppBar(
        title: const Text('会话'),
        actions: [
          IconButton(
            tooltip: '账户设置',
            icon: const Icon(Icons.account_circle_outlined),
            onPressed: () => _openAccountSettings(context),
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: () => _openNewChat(context),
        icon: const Icon(Icons.chat_bubble_outline),
        label: const Text('新对话'),
      ),
      body: Column(
        children: [
          if (!isConnected)
            _MacOfflineBanner(state: connectionAsync.asData?.value),
          Expanded(
            child: threadsAsync.when(
              loading: () => const Center(child: CircularProgressIndicator()),
              error: (e, _) => Center(child: Text('加载失败: $e')),
              data: (list) => RefreshIndicator(
                onRefresh: () =>
                    ref.read(threadListProvider.notifier).refresh(),
                child: list.isEmpty
                    ? ListView(
                        children: const [
                          SizedBox(height: 200),
                          Center(child: Text('暂无会话')),
                        ],
                      )
                    : ListView.builder(
                        itemCount: list.length,
                        itemBuilder: (_, i) => ThreadListTile(
                          summary: list[i],
                          onTap: () => Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              builder: (_) =>
                                  ThreadViewPage(threadId: list[i].threadId),
                            ),
                          ),
                        ),
                      ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  void _openNewChat(BuildContext context) {
    Navigator.of(
      context,
    ).push(MaterialPageRoute<void>(builder: (_) => const ThreadViewPage()));
  }

  void _openAccountSettings(BuildContext context) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(builder: (_) => const AccountSettingsPage()),
    );
  }
}

/// Sticky banner shown above the thread list whenever the WS is not in
/// the [ConnectionState_Connected] state. The text disambiguates between
/// reconnecting / offline / pairing so the user knows whether the daemon
/// is being chased.
class _MacOfflineBanner extends StatelessWidget {
  const _MacOfflineBanner({required this.state});

  final ConnectionState? state;

  String _label() {
    return switch (state) {
      ConnectionState_Reconnecting(:final attempt) => 'Mac 正在重连… (#$attempt)',
      ConnectionState_Pairing() => '配对中…',
      ConnectionState_Disconnected() => 'Mac 已离线',
      _ => 'Mac 已离线',
    };
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Material(
      color: theme.colorScheme.errorContainer,
      child: SafeArea(
        bottom: false,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(
            children: [
              Icon(
                Icons.cloud_off_outlined,
                size: 18,
                color: theme.colorScheme.onErrorContainer,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  _label(),
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: theme.colorScheme.onErrorContainer,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
