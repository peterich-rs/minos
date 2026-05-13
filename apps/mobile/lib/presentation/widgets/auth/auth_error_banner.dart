import 'dart:async';

import 'package:flutter/material.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart' show MinosError;
import 'package:shadcn_ui/shadcn_ui.dart';

/// Auto-dismissing destructive [ShadAlert] driven by an externally-owned
/// auth error object. The 6-second timer matches the Remodex iOS clone — long
/// enough for the user to read the title + detail, short enough not to
/// linger after a successful retry.
///
/// Typed [MinosError] values use the localized Rust-owned copy; unexpected
/// FRB / Dart exceptions fall back to a generic title plus a normalized
/// description so the page never crashes back to Flutter's red error UI.
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
    if (widget.error != null) _arm();
  }

  @override
  void didUpdateWidget(AuthErrorBanner old) {
    super.didUpdateWidget(old);
    // Re-arm on every transition into a non-null error, even if the typed
    // variant is identical to the previous one — repeated identical errors
    // (e.g. two failed login attempts) should re-show the banner.
    if (widget.error != null && widget.error != old.error) {
      _arm();
    } else if (widget.error == null && old.error != null) {
      _timer?.cancel();
      _visible = false;
    }
  }

  void _arm() {
    _timer?.cancel();
    setState(() => _visible = true);
    _timer = Timer(const Duration(seconds: 6), () {
      if (mounted) setState(() => _visible = false);
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
    return ShadAlert.destructive(
      icon: const Icon(Icons.error_outline),
      title: Text(title),
      description: detail == null ? null : Text(detail),
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
