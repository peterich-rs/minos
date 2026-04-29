import 'dart:async';

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

class _QrScannerViewState extends ConsumerState<QrScannerView>
    with WidgetsBindingObserver {
  final MobileScannerController _controller = MobileScannerController(
    autoStart: false,
  );
  String? _lastSubmitted;
  bool _scannerRunning = false;
  bool _viewActive = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        unawaited(_startScanner());
      }
    });
  }

  @override
  void activate() {
    super.activate();
    _viewActive = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        unawaited(_startScanner());
      }
    });
  }

  @override
  void deactivate() {
    _viewActive = false;
    unawaited(_stopScanner());
    super.deactivate();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(_startScanner());
      return;
    }
    unawaited(_stopScanner());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_stopScanner());
    _controller.dispose();
    super.dispose();
  }

  Future<void> _startScanner() async {
    if (!mounted || !_viewActive || _scannerRunning) {
      return;
    }
    try {
      await _controller.start();
      _scannerRunning = true;
    } catch (_) {
      _scannerRunning = false;
    }
  }

  Future<void> _stopScanner() async {
    if (!_scannerRunning) {
      return;
    }
    try {
      await _controller.stop();
    } catch (_) {
      // Lifecycle churn can race with the platform view teardown. We only
      // care that the widget no longer treats the scanner as live.
    } finally {
      _scannerRunning = false;
    }
  }

  @override
  Widget build(BuildContext context) {
    ref.listen<AsyncValue<bool>>(pairingControllerProvider, (_, next) {
      if (next is AsyncError) {
        unawaited(_startScanner());
      }
    });

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
        unawaited(_stopScanner());
        ref.read(pairingControllerProvider.notifier).submit(raw);
      },
    );
  }
}
