import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/presentation/widgets/ui_event_tile.dart';

/// Renders the translated `UiEventMessage` stream for one thread as a
/// flat scrollable list. No chat styling — the spec explicitly pushes the
/// chat UI to a later design pass (plan §D7).
class ThreadViewPage extends ConsumerWidget {
  const ThreadViewPage({super.key, required this.threadId});

  final String threadId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final eventsAsync = ref.watch(threadEventsProvider(threadId));
    return Scaffold(
      appBar: AppBar(title: Text('Thread $threadId')),
      body: eventsAsync.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (e, _) => Center(child: Text('加载失败: $e')),
        data: (events) => events.isEmpty
            ? const Center(child: Text('暂无事件'))
            : ListView.builder(
                itemCount: events.length,
                itemBuilder: (_, i) => UiEventTile(event: events[i]),
              ),
      ),
    );
  }
}
