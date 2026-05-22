import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

/// Thin wrapper around [MobileScanner] that forwards the first non-empty QR
/// payload to the caller.
///
/// The scanner stops while [onDetect] is handling a payload. Returning
/// `true` keeps it stopped so the caller can move to a confirmation step;
/// returning `false` restarts the camera and allows another scan.
class QrScannerView extends StatefulWidget {
  const QrScannerView({required this.onDetect, super.key});

  final FutureOr<bool> Function(String rawValue) onDetect;

  @override
  State<QrScannerView> createState() => _QrScannerViewState();
}

class _QrScannerViewState extends State<QrScannerView>
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
    return MobileScanner(
      fit: BoxFit.cover,
      controller: _controller,
      onDetect: (BarcodeCapture capture) {
        final raw = capture.barcodes.isNotEmpty
            ? capture.barcodes.first.rawValue
            : null;
        if (raw == null || raw.isEmpty) return;
        if (raw == _lastSubmitted) return;
        _lastSubmitted = raw;
        unawaited(_handleDetected(raw));
      },
    );
  }

  Future<void> _handleDetected(String raw) async {
    await _stopScanner();
    bool accepted = false;
    try {
      accepted = await widget.onDetect(raw);
    } catch (_) {
      accepted = false;
    }
    if (!mounted || accepted) return;
    _lastSubmitted = null;
    await _startScanner();
  }
}
