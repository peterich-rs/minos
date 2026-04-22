import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';

/// Debug-only bottom sheet that lets a developer paste a QR JSON payload
/// directly (bypassing the camera). Gated behind [kDebugMode] at the caller
/// so release builds tree-shake it entirely.
class DebugPasteQrSheet extends ConsumerStatefulWidget {
  const DebugPasteQrSheet({super.key});

  @override
  ConsumerState<DebugPasteQrSheet> createState() => _DebugPasteQrSheetState();
}

class _DebugPasteQrSheetState extends ConsumerState<DebugPasteQrSheet> {
  final TextEditingController _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.only(
        left: 16,
        right: 16,
        top: 16,
        bottom: 16 + MediaQuery.of(context).viewInsets.bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Text('粘贴 QR JSON(仅调试)'),
          const SizedBox(height: 8),
          TextField(
            controller: _controller,
            maxLines: 6,
            decoration: const InputDecoration(border: OutlineInputBorder()),
          ),
          const SizedBox(height: 12),
          ShadButton(
            onPressed: () {
              final text = _controller.text.trim();
              if (text.isNotEmpty) {
                ref.read(pairingControllerProvider.notifier).submit(text);
                Navigator.of(context).pop();
              }
            },
            child: const Text('提交'),
          ),
        ],
      ),
    );
  }
}
