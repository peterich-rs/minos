import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/preferred_agent_provider.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/presentation/pages/log_viewer_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/thread_list_tile.dart';
import 'package:minos/src/rust/api/minos.dart';

const String _appVersion = '1.0.0';

/// iOS-flat shell: three tabs (Messages / Agent / Profile) with a sticky
/// large title per tab, a thin top divider, and a Material 3
/// [NavigationBar] themed to match.
class AppShellPage extends ConsumerStatefulWidget {
  const AppShellPage({super.key});

  @override
  ConsumerState<AppShellPage> createState() => _AppShellPageState();
}

class _AppShellPageState extends ConsumerState<AppShellPage> {
  int _tabIndex = 0;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: IndexedStack(
        index: _tabIndex,
        children: const <Widget>[_MessagesTab(), _AgentsTab(), _ProfileTab()],
      ),
      bottomNavigationBar: _BottomNav(
        index: _tabIndex,
        onChanged: (i) => setState(() => _tabIndex = i),
      ),
    );
  }
}

// ─────────────────────────── Messages tab ───────────────────────────

class _MessagesTab extends ConsumerWidget {
  const _MessagesTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final threadsAsync = ref.watch(threadListProvider);
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final isConnected = connection is ConnectionState_Connected;
    final preferredAgent = ref.watch(preferredAgentProvider);

    return SafeArea(
      bottom: false,
      child: Column(
        children: <Widget>[
          _LargeTitleBar(
            title: '消息',
            trailing: _CircleIconButton(
              icon: CupertinoIcons.square_pencil,
              onTap: () => _openNewChat(context, ref),
              tooltip: '新对话',
            ),
          ),
          if (!isConnected) _OfflineBanner(state: connection),
          Expanded(
            child: threadsAsync.when(
              loading: () => const Center(child: CupertinoActivityIndicator()),
              error: (error, _) => Center(
                child: Text('加载失败: $error', style: theme.textTheme.bodyMedium),
              ),
              data: (threads) => RefreshIndicator(
                onRefresh: () =>
                    ref.read(threadListProvider.notifier).refresh(),
                child: threads.isEmpty
                    ? ListView(
                        physics: const AlwaysScrollableScrollPhysics(),
                        padding: const EdgeInsets.fromLTRB(24, 80, 24, 24),
                        children: <Widget>[
                          _EmptyState(
                            icon: CupertinoIcons.bubble_left_bubble_right,
                            title: '还没有会话',
                            subtitle:
                                '点右上角图标开始新对话，首条消息会通过 ${_agentLabel(preferredAgent)} 发起。',
                          ),
                        ],
                      )
                    : _ThreadList(threads: threads),
              ),
            ),
          ),
        ],
      ),
    );
  }

  void _openNewChat(BuildContext context, WidgetRef ref) {
    ref.read(activeSessionControllerProvider.notifier).reset();
    Navigator.of(
      context,
    ).push(MaterialPageRoute<void>(builder: (_) => const ThreadViewPage()));
  }
}

class _ThreadList extends StatelessWidget {
  const _ThreadList({required this.threads});

  final List<ThreadSummary> threads;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ListView.separated(
      padding: const EdgeInsets.only(bottom: 24),
      itemCount: threads.length,
      separatorBuilder: (_, _) => Divider(
        height: 0.5,
        thickness: 0.5,
        indent: 76,
        endIndent: 0,
        color: theme.dividerColor.withValues(alpha: 0.5),
      ),
      itemBuilder: (_, index) => ThreadListTile(
        summary: threads[index],
        onTap: () => Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadViewPage(threadId: threads[index].threadId),
          ),
        ),
      ),
    );
  }
}

// ─────────────────────────── Agent tab ───────────────────────────

class _AgentsTab extends ConsumerWidget {
  const _AgentsTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preferredAgent = ref.watch(preferredAgentProvider);
    final session = ref.watch(activeSessionControllerProvider);
    final currentSessionAgent = _sessionAgent(session);

    return SafeArea(
      bottom: false,
      child: Column(
        children: <Widget>[
          const _LargeTitleBar(title: 'Agent', subtitle: '为新会话选择默认 Agent'),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
              children: <Widget>[
                _GroupedCard(
                  children: <Widget>[
                    for (
                      var i = 0;
                      i < AgentName.values.length;
                      i++
                    ) ...<Widget>[
                      if (i > 0) const _RowDivider(indent: 56),
                      _AgentRow(
                        agent: AgentName.values[i],
                        selected: preferredAgent == AgentName.values[i],
                        active: currentSessionAgent == AgentName.values[i],
                        onTap: () => ref
                            .read(preferredAgentProvider.notifier)
                            .setAgent(AgentName.values[i]),
                      ),
                    ],
                  ],
                ),
                const SizedBox(height: 12),
                _GroupedFooter(
                  text: '切换默认 Agent 只影响新建会话；已有 thread 会保持当前运行上下文。',
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _AgentRow extends StatelessWidget {
  const _AgentRow({
    required this.agent,
    required this.selected,
    required this.active,
    required this.onTap,
  });

  final AgentName agent;
  final bool selected;
  final bool active;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          children: <Widget>[
            _AgentAvatar(agent: agent, size: 36),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Row(
                    children: <Widget>[
                      Text(
                        _agentLabel(agent),
                        style: theme.textTheme.titleMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      if (active) ...<Widget>[
                        const SizedBox(width: 8),
                        const _RunningDot(),
                      ],
                    ],
                  ),
                  const SizedBox(height: 2),
                  Text(
                    _agentDescription(agent),
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                      height: 1.35,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            if (selected)
              Icon(
                CupertinoIcons.check_mark,
                size: 20,
                color: theme.colorScheme.primary,
              )
            else
              const SizedBox(width: 20),
          ],
        ),
      ),
    );
  }
}

class _RunningDot extends StatelessWidget {
  const _RunningDot();

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 6,
      height: 6,
      decoration: const BoxDecoration(
        color: Color(0xFF34C759),
        shape: BoxShape.circle,
      ),
    );
  }
}

// ─────────────────────────── Profile tab ───────────────────────────

class _ProfileTab extends ConsumerWidget {
  const _ProfileTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final authState = ref.watch(authControllerProvider);
    final email = authState is AuthAuthenticated
        ? authState.account.email
        : '—';
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final theme = Theme.of(context);

    return SafeArea(
      bottom: false,
      child: Column(
        children: <Widget>[
          const _LargeTitleBar(title: '我的'),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
              children: <Widget>[
                _AccountHeader(email: email, connection: connection),
                const SizedBox(height: 16),
                _GroupedCard(
                  children: <Widget>[
                    _SettingsRow(
                      icon: CupertinoIcons.ant,
                      title: 'Devtool',
                      subtitle: '日志与请求追踪',
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => const LogViewerPage(),
                        ),
                      ),
                    ),
                    const _RowDivider(indent: 56),
                    _SettingsRow(
                      icon: CupertinoIcons.qrcode_viewfinder,
                      title: '重新配对',
                      subtitle: '清除当前设备绑定并返回扫码页',
                      onTap: () => _confirmForgetPeer(context, ref),
                    ),
                  ],
                ),
                const SizedBox(height: 16),
                _GroupedCard(
                  children: <Widget>[
                    _SettingsRow(
                      icon: CupertinoIcons.square_arrow_right,
                      title: '退出登录',
                      destructive: true,
                      onTap: () => _confirmLogout(context, ref),
                    ),
                  ],
                ),
                const SizedBox(height: 24),
                Center(
                  child: Text(
                    'Minos · v$_appVersion',
                    style: theme.textTheme.labelSmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _confirmForgetPeer(BuildContext context, WidgetRef ref) async {
    final confirmed = await showCupertinoDialog<bool>(
      context: context,
      builder: (ctx) => CupertinoAlertDialog(
        title: const Text('重新配对'),
        content: const Text('这会清除当前设备绑定并返回扫码页。是否继续？'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('取消'),
          ),
          CupertinoDialogAction(
            isDefaultAction: true,
            onPressed: () => Navigator.of(ctx).pop(true),
            child: const Text('继续'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await ref.read(minosCoreProvider).forgetPeer();
    ref.invalidate(hasPersistedPairingProvider);
  }

  Future<void> _confirmLogout(BuildContext context, WidgetRef ref) async {
    final confirmed = await showCupertinoDialog<bool>(
      context: context,
      builder: (ctx) => CupertinoAlertDialog(
        title: const Text('退出登录'),
        content: const Text('当前账户会话会被清除，确认继续？'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('取消'),
          ),
          CupertinoDialogAction(
            isDestructiveAction: true,
            onPressed: () => Navigator.of(ctx).pop(true),
            child: const Text('退出'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await ref.read(authControllerProvider.notifier).logout();
  }
}

class _AccountHeader extends StatelessWidget {
  const _AccountHeader({required this.email, required this.connection});

  final String email;
  final ConnectionState? connection;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final initial = email.isEmpty || email == '—'
        ? '?'
        : email.substring(0, 1).toUpperCase();
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 16),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Row(
        children: <Widget>[
          Container(
            width: 56,
            height: 56,
            decoration: const BoxDecoration(
              shape: BoxShape.circle,
              gradient: LinearGradient(
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
                colors: <Color>[Color(0xFF5AC8FA), Color(0xFF007AFF)],
              ),
            ),
            alignment: Alignment.center,
            child: Text(
              initial,
              style: const TextStyle(
                color: Colors.white,
                fontSize: 22,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.5,
              ),
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  email,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
                const SizedBox(height: 4),
                _ConnectionLine(state: connection),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ConnectionLine extends StatelessWidget {
  const _ConnectionLine({required this.state});

  final ConnectionState? state;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final (label, color) = switch (state) {
      ConnectionState_Connected() => ('Mac 已连接', const Color(0xFF34C759)),
      ConnectionState_Reconnecting(:final attempt) => (
        '重连中 #$attempt',
        const Color(0xFFFF9500),
      ),
      ConnectionState_Pairing() => ('配对中', const Color(0xFFFF9500)),
      _ => ('Mac 离线', const Color(0xFFFF3B30)),
    };
    return Row(
      children: <Widget>[
        Container(
          width: 6,
          height: 6,
          decoration: BoxDecoration(color: color, shape: BoxShape.circle),
        ),
        const SizedBox(width: 6),
        Text(
          label,
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
      ],
    );
  }
}

class _SettingsRow extends StatelessWidget {
  const _SettingsRow({
    required this.icon,
    required this.title,
    required this.onTap,
    this.subtitle,
    this.destructive = false,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final VoidCallback onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tint = destructive
        ? const Color(0xFFFF3B30)
        : theme.colorScheme.onSurface;
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          children: <Widget>[
            Icon(icon, size: 22, color: tint),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    title,
                    style: theme.textTheme.bodyLarge?.copyWith(color: tint),
                  ),
                  if (subtitle != null) ...<Widget>[
                    const SizedBox(height: 2),
                    Text(
                      subtitle!,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            if (!destructive)
              Icon(
                CupertinoIcons.chevron_right,
                size: 16,
                color: theme.colorScheme.outline,
              ),
          ],
        ),
      ),
    );
  }
}

// ─────────────────────────── Reusable shell ───────────────────────────

class _LargeTitleBar extends StatelessWidget {
  const _LargeTitleBar({required this.title, this.subtitle, this.trailing});

  final String title;
  final String? subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 12, 12, 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: <Widget>[
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  title,
                  style: theme.textTheme.headlineLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                    letterSpacing: -0.5,
                  ),
                ),
                if (subtitle != null) ...<Widget>[
                  const SizedBox(height: 2),
                  Text(
                    subtitle!,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class _CircleIconButton extends StatelessWidget {
  const _CircleIconButton({
    required this.icon,
    required this.onTap,
    this.tooltip,
  });

  final IconData icon;
  final VoidCallback onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return IconButton(
      tooltip: tooltip,
      onPressed: onTap,
      icon: Icon(icon, size: 22),
      style: IconButton.styleFrom(
        foregroundColor: theme.colorScheme.primary,
        minimumSize: const Size(40, 40),
      ),
    );
  }
}

class _OfflineBanner extends StatelessWidget {
  const _OfflineBanner({required this.state});

  final ConnectionState? state;

  String _label() {
    return switch (state) {
      ConnectionState_Reconnecting(:final attempt) => 'Mac 正在重连 #$attempt',
      ConnectionState_Pairing() => '配对中',
      ConnectionState_Disconnected() => 'Mac 已离线',
      _ => 'Mac 已离线',
    };
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.fromLTRB(16, 4, 16, 8),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: const Color(0x14FF9500),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Row(
        children: <Widget>[
          const Icon(
            CupertinoIcons.wifi_slash,
            size: 16,
            color: Color(0xFFFF9500),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              _label(),
              style: const TextStyle(
                color: Color(0xFFC76E00),
                fontSize: 13,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState({
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  final IconData icon;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      children: <Widget>[
        Icon(icon, size: 44, color: theme.colorScheme.outline),
        const SizedBox(height: 14),
        Text(
          title,
          style: theme.textTheme.titleMedium?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: 6),
        Text(
          subtitle,
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
            height: 1.4,
          ),
        ),
      ],
    );
  }
}

class _GroupedCard extends StatelessWidget {
  const _GroupedCard({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ClipRRect(
      borderRadius: BorderRadius.circular(14),
      child: Material(
        color: theme.colorScheme.surface,
        child: Column(children: children),
      ),
    );
  }
}

class _GroupedFooter extends StatelessWidget {
  const _GroupedFooter({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
      child: Text(
        text,
        style: theme.textTheme.labelSmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
          height: 1.4,
        ),
      ),
    );
  }
}

class _RowDivider extends StatelessWidget {
  const _RowDivider({this.indent = 0});

  final double indent;

  @override
  Widget build(BuildContext context) {
    return Divider(
      height: 0.5,
      thickness: 0.5,
      indent: indent,
      color: Theme.of(context).dividerColor.withValues(alpha: 0.5),
    );
  }
}

class _AgentAvatar extends StatelessWidget {
  const _AgentAvatar({required this.agent, this.size = 32});

  final AgentName agent;
  final double size;

  @override
  Widget build(BuildContext context) {
    final (label, gradient) = switch (agent) {
      AgentName.codex => (
        'C',
        const LinearGradient(
          colors: <Color>[Color(0xFF30D158), Color(0xFF248A3D)],
        ),
      ),
      AgentName.claude => (
        'A',
        const LinearGradient(
          colors: <Color>[Color(0xFFFF9F0A), Color(0xFFC93400)],
        ),
      ),
      AgentName.gemini => (
        'G',
        const LinearGradient(
          colors: <Color>[Color(0xFF64D2FF), Color(0xFF0A84FF)],
        ),
      ),
    };
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(shape: BoxShape.circle, gradient: gradient),
      alignment: Alignment.center,
      child: Text(
        label,
        style: TextStyle(
          color: Colors.white,
          fontWeight: FontWeight.w700,
          fontSize: size * 0.42,
          letterSpacing: 0.3,
        ),
      ),
    );
  }
}

class _BottomNav extends StatelessWidget {
  const _BottomNav({required this.index, required this.onChanged});

  final int index;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Material(
      color: theme.colorScheme.surface,
      shape: Border(
        top: BorderSide(
          color: theme.dividerColor.withValues(alpha: 0.4),
          width: 0.5,
        ),
      ),
      child: SafeArea(
        top: false,
        child: SizedBox(
          height: 56,
          child: Row(
            children: <Widget>[
              _NavItem(
                icon: CupertinoIcons.bubble_left_bubble_right,
                activeIcon: CupertinoIcons.bubble_left_bubble_right_fill,
                label: '消息',
                selected: index == 0,
                onTap: () => onChanged(0),
              ),
              _NavItem(
                icon: CupertinoIcons.cube,
                activeIcon: CupertinoIcons.cube_fill,
                label: 'Agent',
                selected: index == 1,
                onTap: () => onChanged(1),
              ),
              _NavItem(
                icon: CupertinoIcons.person,
                activeIcon: CupertinoIcons.person_fill,
                label: '我的',
                selected: index == 2,
                onTap: () => onChanged(2),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _NavItem extends StatelessWidget {
  const _NavItem({
    required this.icon,
    required this.activeIcon,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final IconData icon;
  final IconData activeIcon;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final color = selected
        ? theme.colorScheme.primary
        : theme.colorScheme.onSurfaceVariant;
    return Expanded(
      child: InkWell(
        onTap: onTap,
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: <Widget>[
            Icon(selected ? activeIcon : icon, size: 24, color: color),
            const SizedBox(height: 2),
            Text(
              label,
              style: TextStyle(
                fontSize: 11,
                color: color,
                fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

// ─────────────────────────── helpers ───────────────────────────

Color _scaffoldBg(BuildContext context) {
  final isDark = Theme.of(context).brightness == Brightness.dark;
  return isDark ? const Color(0xFF000000) : const Color(0xFFF2F2F7);
}

String _agentLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
  };
}

String _agentDescription(AgentName agent) {
  return switch (agent) {
    AgentName.codex => '代码生成与仓库内编辑，作为主开发代理。',
    AgentName.claude => '长上下文解释与复杂输入整理，偏分析型。',
    AgentName.gemini => '快速探索与多轮补充，可作为新建 thread 入口。',
  };
}

AgentName? _sessionAgent(ActiveSession session) {
  return switch (session) {
    SessionStarting(:final agent) => agent,
    SessionStreaming(:final agent) => agent,
    SessionAwaitingInput(:final agent) => agent,
    _ => null,
  };
}
