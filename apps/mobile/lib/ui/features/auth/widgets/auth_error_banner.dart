import 'dart:async';

import 'package:flutter/material.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart' show MinosError;
import 'package:minos/ui/theme/theme.dart';

/// Auto-dismissing destructive banner for auth errors.
///
/// Visibility is local widget state (not a Riverpod provider) so arming from
/// [initState] / [didUpdateWidget] cannot hit "modify provider while building".
class AuthErrorBanner extends StatefulWidget {
  const AuthErrorBanner({super.key, required this.error});

  final Object? error;

  @override
  State<AuthErrorBanner> createState() => _AuthErrorBannerState();
}

class _AuthErrorBannerState extends State<AuthErrorBanner> {
  Timer? _timer;
  bool _visible = false;

  @override
  void initState() {
    super.initState();
    if (widget.error != null) {
      _arm();
    }
  }

  @override
  void didUpdateWidget(AuthErrorBanner old) {
    super.didUpdateWidget(old);
    if (widget.error != null && widget.error != old.error) {
      _arm();
    } else if (widget.error == null && old.error != null) {
      _timer?.cancel();
      _setVisible(false);
    }
  }

  void _arm() {
    _timer?.cancel();
    _setVisible(true);
    _timer = Timer(const Duration(seconds: 6), () {
      if (mounted) {
        _setVisible(false);
      }
    });
  }

  /// Defer state writes out of the build/update pipeline.
  void _setVisible(bool visible) {
    if (_visible == visible) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _visible == visible) return;
      setState(() => _visible = visible);
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final err = widget.error;
    if (!_visible || err == null) return const SizedBox.shrink();

    final title = switch (err) {
      final MinosError typed => typed.userMessage(),
      _ when _normalizedFallbackDetail(err).contains('CryptoProvider') =>
        'TLS 初始化失败',
      _ => '操作失败',
    };
    final detail = switch (err) {
      final MinosError typed => typed.detail,
      _ => _normalizedFallbackDetail(err),
    };

    final colors = context.minosColors;
    final theme = Theme.of(context);

    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.dangerSoft,
        borderRadius: MinosRadii.smAll,
        border: Border.all(color: colors.danger.withValues(alpha: 0.25)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(MinosSpacing.md),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Icon(Icons.error_outline_rounded, size: 20, color: colors.danger),
            const SizedBox(width: MinosSpacing.sm),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    title,
                    style: theme.textTheme.titleSmall?.copyWith(
                      color: colors.danger,
                    ),
                  ),
                  if (detail != null && detail.isNotEmpty) ...<Widget>[
                    const SizedBox(height: MinosSpacing.xs),
                    Text(
                      detail,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: colors.textSecondary,
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

String _normalizedFallbackDetail(Object error) {
  var text = error.toString().trim();
  if (text.startsWith('PanicException(') && text.endsWith(')')) {
    text = text.substring('PanicException('.length, text.length - 1).trim();
  }

  return text.replaceAll(RegExp(r'\s+'), ' ');
}
