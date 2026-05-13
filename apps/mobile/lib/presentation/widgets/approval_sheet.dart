import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

/// Data class representing an approval request received from the host via
/// the server relay. Maps to `EventKind::ApprovalRequest` on the wire.
class ApprovalRequestData {
  const ApprovalRequestData({
    required this.threadId,
    required this.turnId,
    required this.requestId,
    required this.method,
    required this.params,
    required this.timeoutMs,
  });

  final String threadId;
  final String turnId;
  final String requestId;
  final String method;
  final Map<String, dynamic> params;
  final int timeoutMs;

  /// Parse from a raw JSON payload (e.g. from `UiEventMessage.raw`).
  factory ApprovalRequestData.fromJson(Map<String, dynamic> json) {
    return ApprovalRequestData(
      threadId: json['thread_id'] as String? ?? '',
      turnId: json['turn_id'] as String? ?? '',
      requestId: json['request_id'] as String? ?? '',
      method: json['method'] as String? ?? '',
      params: switch (json['params']) {
        final Map<Object?, Object?> value => value.map(
          (key, value) => MapEntry('$key', value),
        ),
        _ => const <String, dynamic>{},
      },
      timeoutMs: switch (json['timeout_ms']) {
        final int value => value,
        final num value => value.toInt(),
        _ => 120000,
      },
    );
  }
}

/// Modal bottom sheet that displays an approval request and lets the user
/// accept or decline. Shows a countdown timer for the remaining time before
/// auto-decline.
///
/// Usage:
/// ```dart
/// final decision = await showApprovalSheet(context, request: data);
/// ```
Future<ApprovalDecision?> showApprovalSheet(
  BuildContext context, {
  required ApprovalRequestData request,
}) {
  return showModalBottomSheet<ApprovalDecision>(
    context: context,
    isScrollControlled: true,
    isDismissible: false,
    enableDrag: false,
    backgroundColor: Colors.transparent,
    builder: (_) => _ApprovalSheet(request: request),
  );
}

/// The user's decision on an approval request.
enum ApprovalDecision { accept, decline }

class _ApprovalSheet extends StatefulWidget {
  const _ApprovalSheet({required this.request});

  final ApprovalRequestData request;

  @override
  State<_ApprovalSheet> createState() => _ApprovalSheetState();
}

class _ApprovalSheetState extends State<_ApprovalSheet> {
  late final Timer _countdownTimer;
  late int _remainingSeconds;

  @override
  void initState() {
    super.initState();
    _remainingSeconds = (widget.request.timeoutMs / 1000).ceil();
    _countdownTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;
      setState(() {
        _remainingSeconds--;
        if (_remainingSeconds <= 0) {
          _countdownTimer.cancel();
          // Timeout reached — dismiss without a decision (host auto-declines)
          Navigator.of(context).pop(null);
        }
      });
    });
  }

  @override
  void dispose() {
    _countdownTimer.cancel();
    super.dispose();
  }

  String get _requestTypeLabel {
    final method = widget.request.method;
    if (method.contains('command_execution') ||
        method.contains('exec_command')) {
      return '命令执行';
    } else if (method.contains('file_change') ||
        method.contains('apply_patch')) {
      return '文件修改';
    } else if (method.contains('permissions')) {
      return '权限请求';
    }
    return '操作审批';
  }

  IconData get _requestTypeIcon {
    final method = widget.request.method;
    if (method.contains('command_execution') ||
        method.contains('exec_command')) {
      return LucideIcons.terminal;
    } else if (method.contains('file_change') ||
        method.contains('apply_patch')) {
      return LucideIcons.fileDiff;
    } else if (method.contains('permissions')) {
      return LucideIcons.shield;
    }
    return LucideIcons.circleAlert;
  }

  Widget _buildDetailSection(Map<String, dynamic> params) {
    final widgets = <Widget>[];
    final method = widget.request.method;

    // Command execution: show the command
    if (method.contains('command_execution') ||
        method.contains('exec_command')) {
      final command = params['command'] as String? ??
          (params['command_line'] as List?)?.join(' ') ??
          '';
      if (command.isNotEmpty) {
        widgets.add(_DetailRow(label: '命令', value: command, isCode: true));
      }
      final cwd = params['cwd'] as String? ?? params['working_dir'] as String?;
      if (cwd != null && cwd.isNotEmpty) {
        widgets.add(_DetailRow(label: '目录', value: cwd));
      }
    }

    // File change: show affected files
    if (method.contains('file_change') || method.contains('apply_patch')) {
      final files = params['files'] as List? ?? params['patches'] as List?;
      if (files != null && files.isNotEmpty) {
        final fileNames = files
            .map((f) => f is Map ? (f['path'] ?? f['file'] ?? '') : '$f')
            .where((s) => s.toString().isNotEmpty)
            .take(5)
            .toList();
        if (fileNames.isNotEmpty) {
          widgets.add(
            _DetailRow(label: '文件', value: fileNames.join('\n'), isCode: true),
          );
        }
        if (files.length > 5) {
          widgets.add(
            Padding(
              padding: const EdgeInsets.only(left: 60),
              child: Text(
                '…及其他 ${files.length - 5} 个文件',
                style: const TextStyle(fontSize: 12, color: Colors.grey),
              ),
            ),
          );
        }
      }
    }

    // Permissions: show what's being requested
    if (method.contains('permissions')) {
      final permissions =
          params['permissions'] as List? ?? params['scopes'] as List?;
      if (permissions != null && permissions.isNotEmpty) {
        widgets.add(
          _DetailRow(
            label: '权限',
            value: permissions.map((p) => '$p').join(', '),
          ),
        );
      }
    }

    // Reason field (common across types)
    final reason = params['reason'] as String? ?? params['message'] as String?;
    if (reason != null && reason.isNotEmpty) {
      widgets.add(_DetailRow(label: '原因', value: reason));
    }

    // Fallback: show raw JSON if no structured fields were rendered
    if (widgets.isEmpty && params.isNotEmpty) {
      final prettyJson = const JsonEncoder.withIndent('  ').convert(params);
      widgets.add(_DetailRow(label: '详情', value: prettyJson, isCode: true));
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: widgets,
    );
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final theme = Theme.of(context);
    final timerColor = _remainingSeconds <= 30
        ? shadTheme.colorScheme.destructive
        : shadTheme.colorScheme.mutedForeground;

    return Container(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.background,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(16)),
      ),
      padding: EdgeInsets.only(
        left: 20,
        right: 20,
        top: 20,
        bottom: 20 + MediaQuery.of(context).viewPadding.bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // Header
          Row(
            children: [
              Icon(
                _requestTypeIcon,
                size: 22,
                color: shadTheme.colorScheme.primary,
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  _requestTypeLabel,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                    color: shadTheme.colorScheme.foreground,
                  ),
                ),
              ),
              // Countdown badge
              Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 8,
                  vertical: 4,
                ),
                decoration: BoxDecoration(
                  color: timerColor.withValues(alpha: 0.1),
                  borderRadius: BorderRadius.circular(12),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(LucideIcons.clock, size: 14, color: timerColor),
                    const SizedBox(width: 4),
                    Text(
                      '${_remainingSeconds}s',
                      style: TextStyle(
                        fontSize: 13,
                        fontWeight: FontWeight.w500,
                        color: timerColor,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: 16),

          // Details
          _buildDetailSection(widget.request.params),
          const SizedBox(height: 20),

          // Action buttons
          Row(
            children: [
              Expanded(
                child: ShadButton.outline(
                  onPressed: () =>
                      Navigator.of(context).pop(ApprovalDecision.decline),
                  child: const Text('拒绝'),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: ShadButton(
                  onPressed: () =>
                      Navigator.of(context).pop(ApprovalDecision.accept),
                  child: const Text('允许'),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _DetailRow extends StatelessWidget {
  const _DetailRow({
    required this.label,
    required this.value,
    this.isCode = false,
  });

  final String label;
  final String value;
  final bool isCode;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 50,
            child: Text(
              label,
              style: TextStyle(
                fontSize: 13,
                color: shadTheme.colorScheme.mutedForeground,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: isCode
                ? Container(
                    padding: const EdgeInsets.all(8),
                    decoration: BoxDecoration(
                      color: shadTheme.colorScheme.muted,
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Text(
                      value,
                      style: TextStyle(
                        fontSize: 12,
                        fontFamily: 'monospace',
                        color: shadTheme.colorScheme.foreground,
                      ),
                      maxLines: 8,
                      overflow: TextOverflow.ellipsis,
                    ),
                  )
                : Text(
                    value,
                    style: TextStyle(
                      fontSize: 13,
                      color: shadTheme.colorScheme.foreground,
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}
