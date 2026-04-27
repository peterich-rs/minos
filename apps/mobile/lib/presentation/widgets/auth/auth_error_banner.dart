import 'dart:async';

import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart' show MinosError;

/// Auto-dismissing destructive [ShadAlert] driven by an externally-owned
/// [MinosError]. The 6-second timer matches the Remodex iOS clone — long
/// enough for the user to read the title + detail, short enough not to
/// linger after a successful retry.
///
/// Title is the localized [MinosErrorDisplay.userMessage]; description is
/// the dynamic [MinosErrorDisplay.detail] payload (omitted when the
/// variant has no attached data).
class AuthErrorBanner extends StatefulWidget {
  const AuthErrorBanner({super.key, required this.error});

  final MinosError? error;

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
    final detail = err.detail;
    return ShadAlert.destructive(
      icon: const Icon(Icons.error_outline),
      title: Text(err.userMessage()),
      description: detail == null ? null : Text(detail),
    );
  }
}
