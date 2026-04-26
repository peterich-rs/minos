import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/log_records_provider.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Scrollable view of the most recent Rust-side tracing events.
///
/// Auto-scrolls to the tail whenever a new record arrives so the latest
/// failure is always in view. Each row shows level + target + message;
/// long-press copies the line to the clipboard for sharing.
class LogPanel extends ConsumerStatefulWidget {
  const LogPanel({super.key, this.height = 240});

  /// Visible height of the scroll area. Caller controls overall sizing.
  final double height;

  @override
  ConsumerState<LogPanel> createState() => _LogPanelState();
}

class _LogPanelState extends ConsumerState<LogPanel> {
  final ScrollController _controller = ScrollController();
  int _previousLength = 0;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final records = ref.watch(LogRecords.provider);

    // Stick to the tail when a new record lands AND the user was already
    // near the bottom; don't yank scroll out from under them mid-read.
    if (records.length != _previousLength) {
      _previousLength = records.length;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!_controller.hasClients) return;
        final position = _controller.position;
        final atBottom = position.pixels >= position.maxScrollExtent - 32;
        if (atBottom) {
          _controller.jumpTo(position.maxScrollExtent);
        }
      });
    }

    if (records.isEmpty) {
      return SizedBox(
        height: widget.height,
        child: const Center(
          child: Text('暂无日志', style: TextStyle(color: Colors.grey)),
        ),
      );
    }

    return SizedBox(
      height: widget.height,
      child: Scrollbar(
        controller: _controller,
        child: ListView.builder(
          controller: _controller,
          padding: const EdgeInsets.symmetric(vertical: 4),
          itemCount: records.length,
          itemBuilder: (_, i) => _LogRow(record: records[i]),
        ),
      ),
    );
  }
}

class _LogRow extends StatelessWidget {
  const _LogRow({required this.record});

  final LogRecord record;

  @override
  Widget build(BuildContext context) {
    final color = _colorForLevel(record.level);
    final label = _shortLevel(record.level);
    final ts = DateTime.fromMillisecondsSinceEpoch(
      record.tsMs.toInt(),
      isUtc: false,
    );
    final hh = ts.hour.toString().padLeft(2, '0');
    final mm = ts.minute.toString().padLeft(2, '0');
    final ss = ts.second.toString().padLeft(2, '0');
    final ms = ts.millisecond.toString().padLeft(3, '0');
    final time = '$hh:$mm:$ss.$ms';

    final line = '$time  $label  ${record.target}  ${record.message}';

    return InkWell(
      onLongPress: () {
        Clipboard.setData(ClipboardData(text: line));
        ShadToaster.of(
          context,
        ).show(const ShadToast(description: Text('已复制到剪贴板')));
      },
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
        child: RichText(
          text: TextSpan(
            style: const TextStyle(
              fontFamily: 'Menlo',
              fontSize: 11,
              color: Colors.black87,
              height: 1.3,
            ),
            children: <TextSpan>[
              TextSpan(text: '$time  '),
              TextSpan(
                text: label,
                style: TextStyle(color: color, fontWeight: FontWeight.bold),
              ),
              TextSpan(text: '  ${record.target}\n'),
              TextSpan(
                text: '    ${record.message}',
                style: const TextStyle(color: Colors.black54),
              ),
            ],
          ),
        ),
      ),
    );
  }

  static String _shortLevel(LogLevel level) {
    switch (level) {
      case LogLevel.trace:
        return 'TRC';
      case LogLevel.debug:
        return 'DBG';
      case LogLevel.info:
        return 'INF';
      case LogLevel.warn:
        return 'WRN';
      case LogLevel.error:
        return 'ERR';
    }
  }

  static Color _colorForLevel(LogLevel level) {
    switch (level) {
      case LogLevel.trace:
        return Colors.grey;
      case LogLevel.debug:
        return Colors.blueGrey;
      case LogLevel.info:
        return Colors.blue;
      case LogLevel.warn:
        return Colors.orange;
      case LogLevel.error:
        return Colors.red;
    }
  }
}
