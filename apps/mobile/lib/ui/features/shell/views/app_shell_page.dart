import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
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
        destinations: const <NavigationDestination>[
          NavigationDestination(
            icon: Icon(CupertinoIcons.chat_bubble_2),
            selectedIcon: Icon(CupertinoIcons.chat_bubble_2_fill),
            label: '消息',
          ),
          NavigationDestination(
            icon: Icon(CupertinoIcons.desktopcomputer),
            selectedIcon: Icon(CupertinoIcons.desktopcomputer),
            label: 'Hosts',
          ),
          NavigationDestination(
            icon: Icon(CupertinoIcons.person),
            selectedIcon: Icon(CupertinoIcons.person_fill),
            label: '账户',
          ),
        ],
      ),
    );
  }
}
