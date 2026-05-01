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
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/thread_list_tile.dart';
import 'package:minos/src/rust/api/minos.dart';

const String _appVersion = '1.0.0';

/// iOS-flat shell: three tabs (Messages / Partners / Profile) with a sticky
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
            builder: (_) => ThreadViewPage(
              threadId: threads[index].threadId,
              agent: threads[index].agent,
            ),
          ),
        ),
      ),
    );
  }
}

// ─────────────────────────── Partners tab ───────────────────────────

class _AgentsTab extends ConsumerWidget {
  const _AgentsTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final macsAsync = ref.watch(pairedMacsProvider);
    final activeAsync = ref.watch(activeMacProvider);
    final connection = ref.watch(connectionStateProvider).asData?.value;

    return SafeArea(
      bottom: false,
      child: Column(
        children: <Widget>[
          const _LargeTitleBar(title: '伙伴', subtitle: '真人伙伴与已添加的 runtime 设备'),
          Expanded(
            child: RefreshIndicator(
              onRefresh: () async {
                ref.invalidate(pairedMacsProvider);
                await ref.read(pairedMacsProvider.future);
                await ref.read(activeMacProvider.notifier).refresh();
              },
              child: ListView(
                physics: const AlwaysScrollableScrollPhysics(),
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
                children: <Widget>[
                  const _SectionLabel('真人伙伴'),
                  const _PartnerEmptyCard(
                    icon: CupertinoIcons.person_2,
                    title: '还没有真人伙伴',
                    subtitle: '后端尚未开放真人伙伴列表；当前可以先添加 runtime 设备。',
                  ),
                  const SizedBox(height: 18),
                  const _SectionLabel('设备'),
                  macsAsync.when(
                    loading: () => const _PartnerLoadingCard(),
                    error: (error, _) => _PartnerEmptyCard(
                      icon: CupertinoIcons.exclamationmark_triangle,
                      title: '设备状态读取失败',
                      subtitle: error.toString(),
                    ),
                    data: (macs) {
                      if (macs.isEmpty) {
                        return _GroupedCard(
                          children: <Widget>[
                            _AddPartnerRow(
                              onTap: () => Navigator.of(context).push(
                                MaterialPageRoute<void>(
                                  builder: (_) => const PairingPage(),
                                ),
                              ),
                            ),
                          ],
                        );
                      }
                      final activeMacId = activeAsync.asData?.value;
                      return _GroupedCard(
                        children: <Widget>[
                          for (var i = 0; i < macs.length; i++) ...<Widget>[
                            if (i > 0) const _RowDivider(indent: 56),
                            _MacRow(
                              mac: macs[i],
                              isActive:
                                  activeMacId != null &&
                                  activeMacId == macs[i].macDeviceId,
                              connection: connection,
                              onTap: () async {
                                final wasActive =
                                    activeMacId != null &&
                                    activeMacId == macs[i].macDeviceId;
                                if (wasActive) {
                                  // Already active: open the runtime
                                  // detail page so the user can browse
                                  // CLIs, start a session, or remove this
                                  // partner.
                                  Navigator.of(context).push(
                                    MaterialPageRoute<void>(
                                      builder: (_) =>
                                          const _RuntimeProfilePage(),
                                    ),
                                  );
                                } else {
                                  await ref
                                      .read(activeMacProvider.notifier)
                                      .setActive(macs[i].macDeviceId);
                                }
                              },
                              onForget: () =>
                                  _confirmForgetMac(context, ref, macs[i]),
                            ),
                          ],
                          const _RowDivider(indent: 56),
                          _AddPartnerRow(
                            onTap: () => Navigator.of(context).push(
                              MaterialPageRoute<void>(
                                builder: (_) => const PairingPage(),
                              ),
                            ),
                          ),
                        ],
                      );
                    },
                  ),
                  const SizedBox(height: 12),
                  const _GroupedFooter(
                    text: '设备伙伴承载 Agent runtime；点击设备切换路由目标，向左滑动可删除伙伴。',
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _RuntimeProfilePage extends ConsumerWidget {
  const _RuntimeProfilePage();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final agents = ref.watch(runtimeAgentDescriptorsProvider);
    final preferredAgent = ref.watch(preferredAgentProvider);
    final session = ref.watch(activeSessionControllerProvider);
    final currentSessionAgent = _sessionAgent(session);

    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            _LargeTitleBar(
              title: '设备主页',
              subtitle: 'Agent runtime',
              trailing: _CircleIconButton(
                icon: CupertinoIcons.xmark,
                onTap: () => Navigator.of(context).pop(),
                tooltip: '关闭',
              ),
            ),
            Expanded(
              child: RefreshIndicator(
                onRefresh: () async {
                  ref.invalidate(runtimeAgentDescriptorsProvider);
                  await ref.read(runtimeAgentDescriptorsProvider.future);
                },
                child: ListView(
                  physics: const AlwaysScrollableScrollPhysics(),
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
                  children: <Widget>[
                    _RuntimeProfileHeader(connection: connection),
                    const SizedBox(height: 14),
                    Row(
                      children: <Widget>[
                        Expanded(
                          child: FilledButton.icon(
                            onPressed: () => _openNewChat(context, ref),
                            icon: const Icon(CupertinoIcons.bubble_left),
                            label: const Text('发起会话'),
                          ),
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: OutlinedButton.icon(
                            onPressed: () =>
                                _confirmForgetActiveMac(context, ref),
                            icon: const Icon(CupertinoIcons.delete),
                            label: const Text('删除伙伴'),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 22),
                    const _SectionLabel('支持的 Agent'),
                    agents.when(
                      loading: () => const _PartnerLoadingCard(),
                      error: (error, _) => _PartnerEmptyCard(
                        icon: CupertinoIcons.exclamationmark_triangle,
                        title: 'Agent 检测失败',
                        subtitle: error.toString(),
                      ),
                      data: (items) {
                        if (items.isEmpty) {
                          return const _PartnerEmptyCard(
                            icon: CupertinoIcons.command,
                            title: '没有检测到 Agent CLI',
                            subtitle: 'runtime detect 未返回可展示的 CLI。',
                          );
                        }
                        return _GroupedCard(
                          children: <Widget>[
                            for (var i = 0; i < items.length; i++) ...<Widget>[
                              if (i > 0) const _RowDivider(indent: 56),
                              _RuntimeAgentRow(
                                descriptor: items[i],
                                selected: preferredAgent == items[i].name,
                                active: currentSessionAgent == items[i].name,
                                onTap: _agentAvailable(items[i])
                                    ? () => ref
                                          .read(preferredAgentProvider.notifier)
                                          .setAgent(items[i].name)
                                    : null,
                              ),
                            ],
                          ],
                        );
                      },
                    ),
                    const SizedBox(height: 12),
                    const _GroupedFooter(
                      text: '选择默认 Agent 只影响新建会话；已有 thread 会保持当前运行上下文。',
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

  void _openNewChat(BuildContext context, WidgetRef ref) {
    ref.read(activeSessionControllerProvider.notifier).reset();
    Navigator.of(
      context,
    ).push(MaterialPageRoute<void>(builder: (_) => const ThreadViewPage()));
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 4, 4, 8),
      child: Text(
        text,
        style: Theme.of(context).textTheme.labelMedium?.copyWith(
          color: Theme.of(context).colorScheme.onSurfaceVariant,
          fontWeight: FontWeight.w700,
        ),
      ),
    );
  }
}

class _PartnerLoadingCard extends StatelessWidget {
  const _PartnerLoadingCard();

  @override
  Widget build(BuildContext context) {
    return const _GroupedCard(
      children: <Widget>[
        Padding(
          padding: EdgeInsets.all(18),
          child: Center(child: CupertinoActivityIndicator()),
        ),
      ],
    );
  }
}

class _PartnerEmptyCard extends StatelessWidget {
  const _PartnerEmptyCard({
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  final IconData icon;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    return _GroupedCard(
      children: <Widget>[
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 20, 18, 20),
          child: _EmptyState(icon: icon, title: title, subtitle: subtitle),
        ),
      ],
    );
  }
}

/// One row per paired Mac in the Partners tab. Tap = set as active routing
/// target; long-press shows a delete confirm. The row mirrors the visual
/// shape of the legacy single-peer `_RuntimePartnerRow`: avatar +
/// name/connection-line. The active Mac gets a checkmark on the right.
///
/// UI gap (deviceshell-MVP): no swipe-to-forget gesture yet — long-press is
/// the only delete affordance. The Add Partner row + RefreshIndicator on
/// the parent ListView cover the other interactions.
class _MacRow extends StatelessWidget {
  const _MacRow({
    required this.mac,
    required this.isActive,
    required this.connection,
    required this.onTap,
    required this.onForget,
  });

  final MacSummaryDto mac;
  final bool isActive;
  final ConnectionState? connection;
  final VoidCallback onTap;
  final VoidCallback onForget;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final name = mac.macDisplayName.trim().isEmpty
        ? 'Agent Runtime'
        : mac.macDisplayName.trim();
    return InkWell(
      onTap: onTap,
      onLongPress: onForget,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          children: <Widget>[
            const _RuntimeAvatar(size: 42),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    name,
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 3),
                  // Only show the live connection indicator on the active
                  // Mac — the WS only carries one upstream session at a
                  // time, so the indicator on inactive rows would be
                  // misleading.
                  if (isActive)
                    _ConnectionLine(state: connection)
                  else
                    Text(
                      '点击切换为当前路由目标',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            if (isActive)
              Icon(
                CupertinoIcons.check_mark,
                size: 20,
                color: theme.colorScheme.primary,
              )
            else
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

/// Trailing row in the partners list: tap to enter [PairingPage]. Mirrors
/// the iOS-flat "Add Account" affordance.
class _AddPartnerRow extends StatelessWidget {
  const _AddPartnerRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          children: <Widget>[
            Container(
              width: 42,
              height: 42,
              decoration: BoxDecoration(
                color: theme.colorScheme.primary.withValues(alpha: 0.12),
                borderRadius: BorderRadius.circular(12),
              ),
              alignment: Alignment.center,
              child: Icon(
                CupertinoIcons.qrcode_viewfinder,
                color: theme.colorScheme.primary,
                size: 22,
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    '添加伙伴',
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                  const SizedBox(height: 3),
                  Text(
                    '扫描 runtime 二维码',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
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

class _RuntimeProfileHeader extends ConsumerWidget {
  const _RuntimeProfileHeader({required this.connection});

  final ConnectionState? connection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final name = _resolvePeerDisplayName(ref);
    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(18),
      ),
      child: Row(
        children: <Widget>[
          const _RuntimeAvatar(size: 62),
          const SizedBox(width: 16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  name,
                  style: theme.textTheme.titleLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 5),
                _ConnectionLine(state: connection),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Resolves a display name for the *active* Mac partner. After ADR-0020
/// the Mac name comes from the server's `account_mac_pairings` row,
/// surfaced via `pairedMacsProvider`. Falls back to the cached
/// `peerDisplayName` (kept for legacy startup transitions) and finally
/// a generic label so the row never renders empty.
String _resolvePeerDisplayName(WidgetRef ref) {
  const fallback = 'Agent Runtime';
  final activeId = ref.watch(activeMacProvider).asData?.value;
  final macs = ref.watch(pairedMacsProvider).asData?.value;
  if (activeId != null && macs != null) {
    final match = macs.where((m) => m.macDeviceId == activeId).toList();
    if (match.isNotEmpty) {
      final trimmed = match.first.macDisplayName.trim();
      if (trimmed.isNotEmpty) return trimmed;
    }
  }
  final cached = ref.watch(peerDisplayNameProvider).asData?.value;
  if (cached == null) return fallback;
  final trimmed = cached.trim();
  return trimmed.isEmpty ? fallback : trimmed;
}

class _RuntimeAvatar extends StatelessWidget {
  const _RuntimeAvatar({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(size * 0.28),
        gradient: const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: <Color>[Color(0xFF64D2FF), Color(0xFF0A84FF)],
        ),
      ),
      alignment: Alignment.center,
      child: Icon(
        CupertinoIcons.desktopcomputer,
        color: Colors.white,
        size: size * 0.48,
      ),
    );
  }
}

class _RuntimeAgentRow extends StatelessWidget {
  const _RuntimeAgentRow({
    required this.descriptor,
    required this.selected,
    required this.active,
    required this.onTap,
  });

  final AgentDescriptor descriptor;
  final bool selected;
  final bool active;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final available = _agentAvailable(descriptor);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          children: <Widget>[
            _AgentAvatar(agent: descriptor.name, size: 36),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Row(
                    children: <Widget>[
                      Text(
                        _agentLabel(descriptor.name),
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
                    _agentDescriptorLine(descriptor),
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: available
                          ? theme.colorScheme.onSurfaceVariant
                          : theme.colorScheme.error,
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
                      title: '添加伙伴',
                      subtitle: '扫描二维码添加 runtime 设备',
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => const PairingPage(),
                        ),
                      ),
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
      ConnectionState_Connected() => ('在线', const Color(0xFF34C759)),
      ConnectionState_Reconnecting(:final attempt) => (
        '重连中 #$attempt',
        const Color(0xFFFF9500),
      ),
      ConnectionState_Pairing() => ('配对中', const Color(0xFFFF9500)),
      _ => ('离线', const Color(0xFFFF3B30)),
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
      ConnectionState_Reconnecting(:final attempt) => '正在重连 #$attempt',
      ConnectionState_Pairing() => '配对中',
      ConnectionState_Disconnected() => '已离线',
      _ => '已离线',
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

/// Forget the Mac currently set as the routing target. Surfaces an error
/// dialog when no Mac is active (post-ADR-0020 forget needs an explicit
/// `mac_device_id`). Used from `_RuntimeProfilePage`'s "delete partner"
/// button.
Future<void> _confirmForgetActiveMac(
  BuildContext context,
  WidgetRef ref,
) async {
  final activeId = ref.read(activeMacProvider).asData?.value;
  final macs = ref.read(pairedMacsProvider).asData?.value ?? const [];
  final mac = activeId == null
      ? null
      : macs.where((m) => m.macDeviceId == activeId).toList();
  if (mac == null || mac.isEmpty) {
    await showCupertinoDialog<void>(
      context: context,
      builder: (ctx) => CupertinoAlertDialog(
        title: const Text('未选中设备伙伴'),
        content: const Text('请回到伙伴列表选择要删除的设备。'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('好'),
          ),
        ],
      ),
    );
    return;
  }
  await _confirmForgetMac(context, ref, mac.first);
}

/// Forget a specific paired Mac. Shared between the row-level long-press
/// and the runtime-profile delete button.
Future<void> _confirmForgetMac(
  BuildContext context,
  WidgetRef ref,
  MacSummaryDto mac,
) async {
  final name = mac.macDisplayName.trim().isEmpty
      ? 'Agent Runtime'
      : mac.macDisplayName.trim();
  final confirmed = await showCupertinoDialog<bool>(
    context: context,
    builder: (ctx) => CupertinoAlertDialog(
      title: const Text('删除设备伙伴'),
      content: Text('删除「$name」后再次使用需要重新扫码配对。'),
      actions: <Widget>[
        CupertinoDialogAction(
          onPressed: () => Navigator.of(ctx).pop(false),
          child: const Text('取消'),
        ),
        CupertinoDialogAction(
          isDestructiveAction: true,
          onPressed: () => Navigator.of(ctx).pop(true),
          child: const Text('删除'),
        ),
      ],
    ),
  );
  if (confirmed != true) return;
  await ref.read(minosCoreProvider).forgetMac(mac.macDeviceId);
  ref.invalidate(pairedMacsProvider);
  ref.invalidate(runtimeAgentDescriptorsProvider);
  await ref.read(activeMacProvider.notifier).refresh();
  if (context.mounted && Navigator.of(context).canPop()) {
    Navigator.of(context).pop();
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
                icon: CupertinoIcons.person_2,
                activeIcon: CupertinoIcons.person_2_fill,
                label: '伙伴',
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

bool _agentAvailable(AgentDescriptor descriptor) {
  return switch (descriptor.status) {
    AgentStatus_Ok() => true,
    _ => false,
  };
}

String _agentDescriptorLine(AgentDescriptor descriptor) {
  final version = descriptor.version;
  final path = descriptor.path;
  final suffix = <String>[
    if (version != null && version.isNotEmpty) version,
    if (path != null && path.isNotEmpty) path,
  ].join(' · ');
  final detail = suffix.isEmpty ? '' : ' · $suffix';
  return switch (descriptor.status) {
    AgentStatus_Ok() => '可用$detail',
    AgentStatus_Missing() => '未安装',
    AgentStatus_Error(:final reason) => '检测异常: $reason',
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
