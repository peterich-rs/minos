import 'dart:convert';

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/pages/permission_denied_page.dart';
import 'package:minos/presentation/widgets/qr_scanner_view.dart';
import 'package:minos/src/rust/api/minos.dart' as core;

/// QR-driven "add partner" flow.
///
/// The camera only discovers a candidate. Pairing is not submitted until the
/// user confirms the detected runtime on the confirmation screen.
class PairingPage extends ConsumerStatefulWidget {
  const PairingPage({super.key});

  @override
  ConsumerState<PairingPage> createState() => _PairingPageState();
}

class _PairingPageState extends ConsumerState<PairingPage> {
  bool _requestedOnce = false;
  _PairingCandidate? _candidate;

  @override
  Widget build(BuildContext context) {
    ref.listen<AsyncValue<bool>>(pairingControllerProvider, (_, next) {
      if (next is! AsyncError) return;
      final err = next.error;
      if (err is core.MinosError) {
        ShadToaster.of(context).show(
          ShadToast.destructive(
            title: Text(err.userMessage()),
            description: err.detail == null ? null : Text(err.detail!),
          ),
        );
      } else if (err != null) {
        ShadToaster.of(context).show(
          ShadToast.destructive(
            title: const Text('添加失败'),
            description: Text(err.toString()),
          ),
        );
      }
    });

    final candidate = _candidate;
    if (candidate != null) {
      return _PairingConfirmation(
        candidate: candidate,
        onCancel: () => Navigator.of(context).pop(),
        onConfirm: () => _confirmCandidate(candidate),
      );
    }

    final permission = ref.watch(cameraPermissionProvider);
    return Scaffold(
      backgroundColor: Colors.black,
      body: permission.when(
        loading: () => const Center(child: ShadProgress()),
        error: (_, _) => const Center(child: ShadProgress()),
        data: (status) => _buildForStatus(status),
      ),
    );
  }

  Widget _buildForStatus(PermissionStatus status) {
    if (status.isPermanentlyDenied) {
      return const PermissionDeniedPage();
    }
    if (status.isGranted || status.isLimited) {
      return _ScannerSurface(onDetected: _handleDetected);
    }

    if (!_requestedOnce) {
      _requestedOnce = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) {
          ref.read(cameraPermissionProvider.notifier).request();
        }
      });
    }
    return const Center(child: ShadProgress());
  }

  bool _handleDetected(String raw) {
    final candidate = _PairingCandidate.tryParse(raw);
    if (candidate == null) {
      ShadToaster.of(context).show(
        ShadToast.destructive(
          title: const Text('二维码不可用'),
          description: const Text('请扫描 Minos runtime 上显示的添加伙伴二维码。'),
        ),
      );
      return false;
    }
    if (candidate.isExpired) {
      ShadToaster.of(context).show(
        ShadToast.destructive(
          title: const Text('二维码已过期'),
          description: const Text('请在 runtime 上刷新二维码后重新扫描。'),
        ),
      );
      return false;
    }
    setState(() => _candidate = candidate);
    return true;
  }

  Future<void> _confirmCandidate(_PairingCandidate candidate) async {
    await ref
        .read(pairingControllerProvider.notifier)
        .submit(candidate.rawJson, displayName: candidate.hostDisplayName);
    final state = ref.read(pairingControllerProvider);
    if (!mounted || state is! AsyncData<bool> || state.value != true) return;
    ref.invalidate(runtimeAgentDescriptorsProvider);
    Navigator.of(context).pop();
  }
}

class _ScannerSurface extends StatelessWidget {
  const _ScannerSurface({required this.onDetected});

  final bool Function(String raw) onDetected;

  @override
  Widget build(BuildContext context) {
    return Stack(
      fit: StackFit.expand,
      children: <Widget>[
        QrScannerView(onDetect: onDetected),
        const _ScannerShade(),
        SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(18, 10, 18, 28),
            child: Column(
              children: <Widget>[
                Row(
                  children: <Widget>[
                    _RoundIconButton(
                      icon: CupertinoIcons.chevron_left,
                      onTap: () => Navigator.of(context).pop(),
                    ),
                    const Spacer(),
                  ],
                ),
                const Spacer(),
                Container(
                  width: 252,
                  height: 252,
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(26),
                    border: Border.all(
                      color: Colors.white.withValues(alpha: 0.82),
                      width: 2,
                    ),
                  ),
                ),
                const SizedBox(height: 26),
                Text(
                  '扫描添加伙伴二维码',
                  style: Theme.of(context).textTheme.titleLarge?.copyWith(
                    color: Colors.white,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 8),
                Text(
                  '将 runtime 上的二维码放入取景框，识别后再确认添加。',
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: Colors.white.withValues(alpha: 0.72),
                    height: 1.35,
                  ),
                ),
                const Spacer(),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

class _ScannerShade extends StatelessWidget {
  const _ScannerShade();

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: <Color>[
              Colors.black.withValues(alpha: 0.66),
              Colors.black.withValues(alpha: 0.12),
              Colors.black.withValues(alpha: 0.72),
            ],
          ),
        ),
      ),
    );
  }
}

class _PairingConfirmation extends ConsumerWidget {
  const _PairingConfirmation({
    required this.candidate,
    required this.onCancel,
    required this.onConfirm,
  });

  final _PairingCandidate candidate;
  final VoidCallback onCancel;
  final Future<void> Function() onConfirm;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pairing = ref.watch(pairingControllerProvider);
    final loading = pairing is AsyncLoading;
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 16, 20, 24),
          child: Column(
            children: <Widget>[
              Row(
                children: <Widget>[
                  _RoundIconButton(
                    icon: CupertinoIcons.xmark,
                    foreground: theme.colorScheme.onSurface,
                    background: theme.colorScheme.surface,
                    onTap: loading ? null : onCancel,
                  ),
                  const Spacer(),
                ],
              ),
              const Spacer(),
              _RuntimeGlyph(name: candidate.hostDisplayName),
              const SizedBox(height: 26),
              Text(
                '添加这个伙伴？',
                style: theme.textTheme.headlineSmall?.copyWith(
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 10),
              Text(
                candidate.hostDisplayName,
                textAlign: TextAlign.center,
                style: theme.textTheme.titleMedium?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 22),
              _InfoCard(candidate: candidate),
              const Spacer(),
              SizedBox(
                width: double.infinity,
                child: FilledButton(
                  onPressed: loading ? null : onConfirm,
                  child: loading
                      ? const CupertinoActivityIndicator()
                      : const Text('确认添加'),
                ),
              ),
              const SizedBox(height: 10),
              SizedBox(
                width: double.infinity,
                child: TextButton(
                  onPressed: loading ? null : onCancel,
                  child: const Text('取消'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _InfoCard extends StatelessWidget {
  const _InfoCard({required this.candidate});

  final _PairingCandidate candidate;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(18),
      ),
      child: Column(
        children: <Widget>[
          _InfoLine(label: '类型', value: candidate.kindLabel),
          const SizedBox(height: 10),
          _InfoLine(label: '二维码', value: candidate.expiryLabel),
        ],
      ),
    );
  }
}

class _InfoLine extends StatelessWidget {
  const _InfoLine({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Row(
      children: <Widget>[
        Text(
          label,
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        const Spacer(),
        Flexible(
          child: Text(
            value,
            textAlign: TextAlign.right,
            style: theme.textTheme.bodyMedium?.copyWith(
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
      ],
    );
  }
}

class _RuntimeGlyph extends StatelessWidget {
  const _RuntimeGlyph({required this.name});

  final String name;

  @override
  Widget build(BuildContext context) {
    final (icon, label, colors) = _runtimeVisual(name);
    return Container(
      width: 116,
      height: 116,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(30),
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: colors,
        ),
        boxShadow: <BoxShadow>[
          BoxShadow(
            color: colors.last.withValues(alpha: 0.26),
            blurRadius: 28,
            offset: const Offset(0, 16),
          ),
        ],
      ),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: <Widget>[
          Icon(icon, color: Colors.white, size: 42),
          const SizedBox(height: 8),
          Text(
            label,
            style: const TextStyle(
              color: Colors.white,
              fontWeight: FontWeight.w800,
              letterSpacing: 0.4,
            ),
          ),
        ],
      ),
    );
  }
}

class _RoundIconButton extends StatelessWidget {
  const _RoundIconButton({
    required this.icon,
    required this.onTap,
    this.foreground = Colors.white,
    this.background,
  });

  final IconData icon;
  final VoidCallback? onTap;
  final Color foreground;
  final Color? background;

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: onTap,
      icon: Icon(icon, size: 20),
      style: IconButton.styleFrom(
        foregroundColor: foreground,
        backgroundColor: background ?? Colors.white.withValues(alpha: 0.14),
        minimumSize: const Size(44, 44),
      ),
    );
  }
}

class _PairingCandidate {
  const _PairingCandidate({
    required this.rawJson,
    required this.hostDisplayName,
    required this.pairingToken,
    required this.expiresAtMs,
  });

  final String rawJson;
  final String hostDisplayName;
  final String pairingToken;
  final int expiresAtMs;

  bool get isExpired {
    return expiresAtMs > 0 &&
        DateTime.now().millisecondsSinceEpoch >= expiresAtMs;
  }

  String get kindLabel {
    final lower = hostDisplayName.toLowerCase();
    if (lower.contains('linux')) return 'Linux runtime';
    if (lower.contains('mac') || lower.contains('darwin')) {
      return 'macOS runtime';
    }
    return 'Agent runtime';
  }

  String get expiryLabel {
    if (expiresAtMs <= 0) return '未声明过期时间';
    final expires = DateTime.fromMillisecondsSinceEpoch(expiresAtMs);
    final minutes = expires.difference(DateTime.now()).inMinutes;
    if (minutes <= 0) return '即将过期';
    return '$minutes 分钟后过期';
  }

  static _PairingCandidate? tryParse(String raw) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, Object?>) return null;
      final version = decoded['v'];
      if (version is num && version.toInt() != 2) return null;
      final host = decoded['host_display_name'] ?? decoded['mac_display_name'];
      final token = decoded['pairing_token'] ?? decoded['token'];
      final expires = decoded['expires_at_ms'];
      if (host is! String || host.trim().isEmpty) return null;
      if (token is! String || token.trim().isEmpty) return null;
      return _PairingCandidate(
        rawJson: raw,
        hostDisplayName: host.trim(),
        pairingToken: token,
        expiresAtMs: expires is num ? expires.toInt() : 0,
      );
    } catch (_) {
      return null;
    }
  }
}

(IconData, String, List<Color>) _runtimeVisual(String name) {
  final lower = name.toLowerCase();
  if (lower.contains('linux')) {
    return (
      Icons.terminal,
      'LINUX',
      const <Color>[Color(0xFF2C2C2E), Color(0xFF111111)],
    );
  }
  if (lower.contains('mac') || lower.contains('darwin')) {
    return (
      CupertinoIcons.desktopcomputer,
      'macOS',
      const <Color>[Color(0xFF64D2FF), Color(0xFF0A84FF)],
    );
  }
  return (
    CupertinoIcons.cube_box,
    'MINOS',
    const <Color>[Color(0xFF30D158), Color(0xFF248A3D)],
  );
}

Color _scaffoldBg(BuildContext context) {
  final isDark = Theme.of(context).brightness == Brightness.dark;
  return isDark ? const Color(0xFF000000) : const Color(0xFFF2F2F7);
}
