import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';

/// Shown when the OS reports [PermissionStatus.permanentlyDenied] — the only
/// recourse is to deep-link the user to the Settings app.
class PermissionDeniedPage extends ConsumerWidget {
  const PermissionDeniedPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text('Minos 需要使用相机扫描 Mac 上的配对二维码'),
              const SizedBox(height: 16),
              ShadButton(
                onPressed: () =>
                    ref.read(cameraPermissionProvider.notifier).openSettings(),
                child: const Text('打开设置'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
