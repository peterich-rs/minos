import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/thread_list_tile.dart';

/// Landing page once pairing completes. Shows the paginated thread list.
class ThreadListPage extends ConsumerWidget {
  const ThreadListPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final threadsAsync = ref.watch(threadListProvider);

    return Scaffold(
      appBar: AppBar(title: const Text('会话')),
      body: threadsAsync.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (e, _) => Center(child: Text('加载失败: $e')),
        data: (list) => RefreshIndicator(
          onRefresh: () => ref.read(threadListProvider.notifier).refresh(),
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
    );
  }
}
