import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/pages/permission_denied_page.dart';
import 'package:minos/presentation/widgets/debug_paste_qr_sheet.dart';
import 'package:minos/presentation/widgets/qr_scanner_view.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Home for the pre-paired device. Drives the camera permission ladder and
/// hosts the QR scanner.
class PairingPage extends ConsumerStatefulWidget {
  const PairingPage({super.key});

  @override
  ConsumerState<PairingPage> createState() => _PairingPageState();
}

class _PairingPageState extends ConsumerState<PairingPage> {
  bool _requestedOnce = false;

  @override
  Widget build(BuildContext context) {
    // Surface MinosError -> destructive toast whenever the submission fails.
    ref.listen<AsyncValue<bool>>(pairingControllerProvider, (_, next) {
      if (next is AsyncError) {
        final err = next.error;
        if (err is MinosError) {
          ShadToaster.of(
            context,
          ).show(ShadToast.destructive(description: Text(err.userMessage())));
        }
      }
    });

    final permission = ref.watch(cameraPermissionProvider);

    return Scaffold(
      body: SafeArea(
        child: permission.when(
          loading: () => const Center(child: ShadProgress()),
          error: (_, _) => const Center(child: ShadProgress()),
          data: (status) => _buildForStatus(context, status),
        ),
      ),
      floatingActionButton: kDebugMode
          ? FloatingActionButton.extended(
              icon: const Icon(Icons.content_paste),
              label: const Text('Paste QR JSON'),
              onPressed: () => showShadSheet<void>(
                context: context,
                builder: (_) => const DebugPasteQrSheet(),
              ),
            )
          : null,
    );
  }

  Widget _buildForStatus(BuildContext context, PermissionStatus status) {
    if (status.isPermanentlyDenied) {
      return const PermissionDeniedPage();
    }
    if (status.isGranted || status.isLimited) {
      return Padding(
        padding: const EdgeInsets.all(16),
        child: ShadCard(
          title: const Text('扫描配对二维码'),
          description: const Text('在手机上扫描 Mac 端显示的二维码'),
          child: const AspectRatio(aspectRatio: 1, child: QrScannerView()),
        ),
      );
    }

    // `denied` on first mount: trigger the OS prompt once, then wait for the
    // notifier to re-emit.
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
}
