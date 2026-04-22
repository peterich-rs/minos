import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';

/// Landing page shown once pairing completes. Displays "已连接 {macName}" in a
/// [ShadCard].
class HomePage extends ConsumerWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final response = ref.watch(pairingControllerProvider).value;
    return Scaffold(
      body: Center(
        child: ShadCard(
          title: const Text('已连接'),
          description: Text(response?.macName ?? ''),
        ),
      ),
    );
  }
}
