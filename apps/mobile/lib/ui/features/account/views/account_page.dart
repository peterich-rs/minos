import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/minos_page_header.dart';
import 'package:minos/ui/core/widgets/minos_status_dot.dart';
import 'package:minos/ui/core/widgets/minos_surface.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/theme/theme.dart';

const String _appVersion = '1.0.0';

/// Minimal account tab for the golden path (email, connection, logout).
class AccountPage extends ConsumerWidget {
  const AccountPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final authState = ref.watch(authControllerProvider);
    final email = authState is AuthAuthenticated
        ? authState.account.email
        : '—';
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final initial = email.isEmpty || email == '—'
        ? '?'
        : email.substring(0, 1).toUpperCase();

    return ColoredBox(
      color: colors.canvas,
      child: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            const MinosPageHeader(title: '账户'),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.fromLTRB(
                  MinosSpacing.pageX,
                  MinosSpacing.sm,
                  MinosSpacing.pageX,
                  MinosSpacing.pageBottom,
                ),
                children: <Widget>[
                  MinosSurface(
                    bordered: true,
                    padding: const EdgeInsets.all(MinosSpacing.lg),
                    child: Row(
                      children: <Widget>[
                        Container(
                          width: 56,
                          height: 56,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: colors.accentSoft,
                          ),
                          alignment: Alignment.center,
                          child: Text(
                            initial,
                            style: theme.textTheme.headlineMedium?.copyWith(
                              color: colors.accent,
                            ),
                          ),
                        ),
                        const SizedBox(width: MinosSpacing.md),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: <Widget>[
                              Text(
                                email,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: theme.textTheme.titleMedium,
                              ),
                              const SizedBox(height: MinosSpacing.xs),
                              _ConnectionLine(state: connection),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: MinosSpacing.lg),
                  MinosSurface(
                    bordered: true,
                    child: Column(
                      children: <Widget>[
                        _SettingsRow(
                          icon: CupertinoIcons.ant,
                          title: '开发者工具',
                          subtitle: '日志与请求追踪',
                          onTap: () => context.push(AppRoutes.logViewer),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: MinosSpacing.lg),
                  MinosSurface(
                    bordered: true,
                    child: _SettingsRow(
                      icon: CupertinoIcons.square_arrow_right,
                      title: '退出登录',
                      destructive: true,
                      onTap: () => _confirmLogout(context, ref),
                    ),
                  ),
                  const SizedBox(height: MinosSpacing.xxl),
                  Center(
                    child: Text(
                      'Minos · v$_appVersion',
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: colors.textTertiary,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
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

class _ConnectionLine extends StatelessWidget {
  const _ConnectionLine({required this.state});

  final ConnectionState? state;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final (label, presence) = switch (state) {
      ConnectionState_Connected() => ('实时连接在线', MinosPresence.online),
      ConnectionState_Reconnecting(:final attempt) => (
        '实时连接重连中 #$attempt',
        MinosPresence.warning,
      ),
      ConnectionState_Pairing() => ('实时连接初始化中', MinosPresence.warning),
      _ => ('实时连接离线', MinosPresence.offline),
    };

    return Row(
      children: <Widget>[
        MinosStatusDot(presence: presence, size: 7),
        const SizedBox(width: MinosSpacing.xs),
        Flexible(
          child: Text(
            label,
            style: theme.textTheme.bodySmall?.copyWith(
              color: colors.textSecondary,
            ),
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
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final tint = destructive ? colors.danger : colors.textPrimary;

    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: MinosSpacing.lg,
            vertical: MinosSpacing.md,
          ),
          child: Row(
            children: <Widget>[
              Icon(icon, size: 22, color: tint),
              const SizedBox(width: MinosSpacing.md),
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
                          color: colors.textSecondary,
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
                  color: colors.textTertiary,
                ),
            ],
          ),
        ),
      ),
    );
  }
}
