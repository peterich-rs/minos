import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import 'package:minos/application/minos_providers.dart';

/// Thin wrapper around [MobileScanner] that forwards the first non-empty QR
/// payload to [pairingControllerProvider].
class QrScannerView extends ConsumerStatefulWidget {
  const QrScannerView({super.key});

  @override
  ConsumerState<QrScannerView> createState() => _QrScannerViewState();
}

class _QrScannerViewState extends ConsumerState<QrScannerView> {
  final MobileScannerController _controller = MobileScannerController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MobileScanner(
      controller: _controller,
      onDetect: (BarcodeCapture capture) {
        final raw = capture.barcodes.isNotEmpty
            ? capture.barcodes.first.rawValue
            : null;
        if (raw != null && raw.isNotEmpty) {
          ref.read(pairingControllerProvider.notifier).submit(raw);
        }
      },
    );
  }
}
