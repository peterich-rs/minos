import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import 'package:minos/application/minos_providers.dart';

/// Thin wrapper around [MobileScanner] that forwards the first non-empty QR
/// payload to [pairingControllerProvider].
///
/// Submits at most once per camera-detected payload while the controller
/// is busy or has already accepted the same raw value, otherwise the
/// scanner would re-fire `submit` every frame and flood both the FRB
/// boundary and the log panel with duplicate pair attempts.
class QrScannerView extends ConsumerStatefulWidget {
  const QrScannerView({super.key});

  @override
  ConsumerState<QrScannerView> createState() => _QrScannerViewState();
}

class _QrScannerViewState extends ConsumerState<QrScannerView> {
  final MobileScannerController _controller = MobileScannerController();
  String? _lastSubmitted;

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
        if (raw == null || raw.isEmpty) return;

        final pairing = ref.read(pairingControllerProvider);
        // Already in flight — let it finish before considering a re-scan.
        if (pairing is AsyncLoading) return;
        // Same QR already accepted: ignore until the user clears it via
        // forgetPeer (which resets _lastSubmitted on next mount).
        if (raw == _lastSubmitted && pairing is AsyncData) return;

        _lastSubmitted = raw;
        ref.read(pairingControllerProvider.notifier).submit(raw);
      },
    );
  }
}
