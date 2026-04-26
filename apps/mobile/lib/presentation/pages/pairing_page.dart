import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/log_records_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/pages/permission_denied_page.dart';
import 'package:minos/presentation/widgets/debug_paste_qr_sheet.dart';
import 'package:minos/presentation/widgets/log_panel.dart';
import 'package:minos/presentation/widgets/qr_scanner_view.dart';
import 'package:minos/src/rust/api/minos.dart' as core;

/// Home for the pre-paired device. Drives the camera permission ladder,
/// hosts the QR scanner, and surfaces the live log tail + most recent
/// pairing error so the user can diagnose connect failures on-device.
class PairingPage extends ConsumerStatefulWidget {
  const PairingPage({super.key});

  @override
  ConsumerState<PairingPage> createState() => _PairingPageState();
}

class _PairingPageState extends ConsumerState<PairingPage> {
  bool _requestedOnce = false;

  @override
  Widget build(BuildContext context) {
    // Surface failures as a destructive toast. Typed core.MinosError gets
    // the localized hint in the title and the dynamic detail (URL / TLS
    // error / HTTP status) in the description so the user can see WHY
    // without opening the log panel. Non-MinosError (e.g. frb
    // PanicException) falls back to its `toString()` so the surface is
    // never silently empty.
    ref.listen<AsyncValue<bool>>(pairingControllerProvider, (_, next) {
      if (next is! AsyncError) return;
      final err = next.error;
      if (err is core.MinosError) {
        final detail = err.detail;
        ShadToaster.of(context).show(
          ShadToast.destructive(
            title: Text(err.userMessage()),
            description: detail == null ? null : Text(detail),
          ),
        );
      } else if (err != null) {
        ShadToaster.of(context).show(
          ShadToast.destructive(
            title: const Text('配对失败'),
            description: Text(err.toString()),
          ),
        );
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
      return _ScannerWithDiagnostics();
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

/// Pairing surface once the camera is available: scanner + connection
/// banner + most-recent error detail + log tail. Sized for a phone, so
/// scrolls vertically on narrow screens.
class _ScannerWithDiagnostics extends ConsumerWidget {
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(pairingControllerProvider);
    final connectionAsync = ref.watch(connectionStateProvider);

    return SingleChildScrollView(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          ShadCard(
            title: const Text('扫描配对二维码'),
            description: const Text('在手机上扫描 Mac 端显示的二维码'),
            child: const AspectRatio(aspectRatio: 1, child: QrScannerView()),
          ),
          const SizedBox(height: 12),
          _ConnectionBanner(
            controller: controller,
            connectionState: connectionAsync,
          ),
          if (controller is AsyncError &&
              controller.error is core.MinosError) ...<Widget>[
            const SizedBox(height: 12),
            _LastErrorCard(error: controller.error! as core.MinosError),
          ],
          const SizedBox(height: 12),
          const _LogPanelCard(),
        ],
      ),
    );
  }
}

class _ConnectionBanner extends StatelessWidget {
  const _ConnectionBanner({
    required this.controller,
    required this.connectionState,
  });

  final AsyncValue<bool> controller;
  final AsyncValue<core.ConnectionState> connectionState;

  @override
  Widget build(BuildContext context) {
    final isLoading = controller is AsyncLoading;
    final state = connectionState.asData?.value;
    final stateLabel = state == null ? '未知' : _describeState(state);

    return ShadCard(
      title: Row(
        children: <Widget>[
          if (isLoading) ...<Widget>[
            const SizedBox(width: 16, height: 16, child: ShadProgress()),
            const SizedBox(width: 8),
          ],
          const Text('连接状态'),
        ],
      ),
      description: Text('当前: $stateLabel'),
    );
  }

  static String _describeState(core.ConnectionState state) {
    return switch (state) {
      core.ConnectionState_Disconnected() => '已断开',
      core.ConnectionState_Pairing() => '配对中',
      core.ConnectionState_Connected() => '已连接',
      final core.ConnectionState_Reconnecting r => '重连中 (#${r.attempt})',
    };
  }
}

class _LastErrorCard extends StatelessWidget {
  const _LastErrorCard({required this.error});

  final core.MinosError error;

  @override
  Widget build(BuildContext context) {
    final detail = error.detail;
    return ShadCard(
      title: Row(
        children: <Widget>[
          const Icon(Icons.error_outline, color: Colors.red, size: 18),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              error.userMessage(),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          IconButton(
            tooltip: '复制错误详情',
            icon: const Icon(Icons.copy, size: 18),
            onPressed: detail == null
                ? null
                : () {
                    Clipboard.setData(ClipboardData(text: detail));
                    ShadToaster.of(
                      context,
                    ).show(const ShadToast(description: Text('已复制到剪贴板')));
                  },
          ),
        ],
      ),
      description: detail == null
          ? const Text('（无附加信息）')
          : SelectableText(
              detail,
              style: const TextStyle(fontFamily: 'Menlo', fontSize: 11),
            ),
    );
  }
}

class _LogPanelCard extends ConsumerWidget {
  const _LogPanelCard();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return ShadCard(
      title: Row(
        children: <Widget>[
          const Text('日志'),
          const Spacer(),
          IconButton(
            tooltip: '清空',
            icon: const Icon(Icons.clear_all, size: 18),
            onPressed: () => ref.read(LogRecords.provider.notifier).clear(),
          ),
        ],
      ),
      description: const Text('Rust 后台事件（最近 500 条；长按复制单行）'),
      child: const LogPanel(height: 260),
    );
  }
}
