import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_highlight/flutter_highlight.dart';
import 'package:flutter_highlight/themes/atom-one-light.dart';

/// Visualises one `UiEventMessage_ToolCallPlaced` (+ its later
/// `ToolCallCompleted`). Collapsed by default — the user only opens it
/// to inspect args/output. The status icon flips through:
///
///   - in-progress  → spinner (no [output], not [isError])
///   - success      → check
///   - failure      → x
class ToolCallCard extends StatelessWidget {
  const ToolCallCard({
    super.key,
    required this.toolCallId,
    required this.toolName,
    required this.argsJson,
    this.output,
    this.isError = false,
  });

  final String toolCallId;
  final String toolName;
  final String argsJson;

  /// Null while the call is still in flight (no
  /// `ToolCallCompleted` seen yet).
  final String? output;

  /// True iff the matching `ToolCallCompleted` carried `isError=true`.
  final bool isError;

  bool get _inFlight => output == null;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Card(
        margin: EdgeInsets.zero,
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(10),
          side: BorderSide(color: theme.colorScheme.outlineVariant),
        ),
        child: Theme(
          data: theme.copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            tilePadding: const EdgeInsets.symmetric(horizontal: 12),
            childrenPadding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
            leading: _StatusIcon(inFlight: _inFlight, isError: isError),
            title: Text(
              toolName,
              style: theme.textTheme.bodyMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            subtitle: Text(
              _inFlight ? 'running…' : (isError ? 'failed' : 'done'),
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
            children: [
              _LabeledBlock(
                label: 'args',
                child: HighlightView(
                  _prettyJson(argsJson),
                  language: 'json',
                  theme: atomOneLightTheme,
                  padding: const EdgeInsets.all(8),
                  textStyle: const TextStyle(
                    fontFamily: 'monospace',
                    fontSize: 12,
                  ),
                ),
              ),
              if (output != null) ...[
                const SizedBox(height: 8),
                _LabeledBlock(
                  label: isError ? 'error' : 'output',
                  child: SelectableText(
                    output!,
                    style: theme.textTheme.bodySmall?.copyWith(
                      fontFamily: 'monospace',
                      color: isError ? theme.colorScheme.error : null,
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  static String _prettyJson(String raw) {
    try {
      final decoded = jsonDecode(raw);
      return const JsonEncoder.withIndent('  ').convert(decoded);
    } catch (_) {
      return raw;
    }
  }
}

class _StatusIcon extends StatelessWidget {
  const _StatusIcon({required this.inFlight, required this.isError});
  final bool inFlight;
  final bool isError;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    if (inFlight) {
      return const SizedBox(
        width: 18,
        height: 18,
        child: CircularProgressIndicator(strokeWidth: 2),
      );
    }
    if (isError) {
      return Icon(Icons.close, size: 18, color: scheme.error);
    }
    return Icon(Icons.check, size: 18, color: scheme.primary);
  }
}

class _LabeledBlock extends StatelessWidget {
  const _LabeledBlock({required this.label, required this.child});
  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          label,
          style: theme.textTheme.labelSmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        const SizedBox(height: 4),
        Container(
          decoration: BoxDecoration(
            color: theme.colorScheme.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(6),
          ),
          padding: const EdgeInsets.all(4),
          child: child,
        ),
      ],
    );
  }
}
