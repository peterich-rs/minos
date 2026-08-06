import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/ui/features/account/views/account_page.dart';
import 'package:minos/ui/features/hosts/views/hosts_page.dart';
import 'package:minos/ui/features/messages/views/messages_page.dart';
import 'package:minos/ui/theme/theme.dart';

/// Golden-path mobile shell: Messages / Hosts / Account.
///
/// Single-column bottom navigation. Agent sessions, projects, and agent-profile
/// management stay reachable via secondary routes.
class AppShellPage extends ConsumerWidget {
  const AppShellPage({super.key});

  static const int messagesTab = 0;
  static const int hostsTab = 1;
  static const int accountTab = 2;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final tabIndex = ref.watch(shellTabIndexProvider).clamp(0, 2);
    final colors = context.minosColors;
    // C6.3: Messages tab badge from Hub inbox unread sum.
    final unread = ref.watch(socialUnreadCountProvider);

    // R4: non-silent subscription limit (LRU eviction / cap).
    ref.listen<SubscriptionLimitNotice?>(subscriptionLimitNoticeProvider, (
      previous,
      next,
    ) {
      if (next == null) return;
      if (previous?.atMs == next.atMs) return;
      final messenger = ScaffoldMessenger.maybeOf(context);
      messenger?.showSnackBar(
        SnackBar(
          content: Text(
            next.limit > 0
                ? '实时订阅已达上限（${next.current}/${next.limit}），部分会话将仅在打开时同步'
                : '实时订阅已达上限，部分会话将仅在打开时同步',
          ),
        ),
      );
    });

    return Scaffold(
      backgroundColor: colors.canvas,
      body: IndexedStack(
        index: tabIndex,
        children: const <Widget>[MessagesPage(), HostsPage(), AccountPage()],
      ),
      bottomNavigationBar: NavigationBar(
        selectedIndex: tabIndex,
        onDestinationSelected: (index) {
          ref.read(shellTabIndexProvider.notifier).select(index);
        },
        destinations: <NavigationDestination>[
          NavigationDestination(
            icon: Badge(
              isLabelVisible: unread > 0,
              label: Text(unread > 99 ? '99+' : '$unread'),
              child: const Icon(CupertinoIcons.chat_bubble_2),
            ),
            selectedIcon: Badge(
              isLabelVisible: unread > 0,
              label: Text(unread > 99 ? '99+' : '$unread'),
              child: const Icon(CupertinoIcons.chat_bubble_2_fill),
            ),
            label: '消息',
          ),
          const NavigationDestination(
            icon: Icon(CupertinoIcons.desktopcomputer),
            selectedIcon: Icon(CupertinoIcons.desktopcomputer),
            label: 'Hosts',
          ),
          const NavigationDestination(
            icon: Icon(CupertinoIcons.person),
            selectedIcon: Icon(CupertinoIcons.person_fill),
            label: '账户',
          ),
        ],
      ),
    );
  }
}
