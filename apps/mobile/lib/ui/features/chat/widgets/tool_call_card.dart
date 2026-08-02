import 'dart:convert';

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_highlight/flutter_highlight.dart';
import 'package:flutter_highlight/themes/atom-one-dark.dart';
import 'package:flutter_highlight/themes/atom-one-light.dart';
import 'package:minos/ui/theme/theme.dart';

/// Collapsed-by-default tool call card for the transcript stream.
class ToolCallCard extends StatefulWidget {
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
  final String? output;
  final bool isError;

  @override
  State<ToolCallCard> createState() => _ToolCallCardState();
}

class _ToolCallCardState extends State<ToolCallCard> {
  bool _expanded = false;

  bool get _inFlight => widget.output == null;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.minosColors;
    final statusLabel = _inFlight
        ? 'running…'
        : (widget.isError ? 'failed' : 'done');
    final statusColor = _inFlight
        ? colors.accent
        : (widget.isError ? colors.danger : colors.success);

    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: MinosSpacing.md,
        vertical: MinosSpacing.xs,
      ),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: colors.surface,
          borderRadius: MinosRadii.smAll,
          border: Border.all(color: colors.borderSubtle),
        ),
        child: Column(
          children: <Widget>[
            Material(
              color: Colors.transparent,
              child: InkWell(
                borderRadius: MinosRadii.smAll,
                onTap: () => setState(() => _expanded = !_expanded),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: MinosSpacing.md,
                    vertical: MinosSpacing.md,
                  ),
                  child: Row(
                    children: <Widget>[
                      _StatusIcon(inFlight: _inFlight, isError: widget.isError),
                      const SizedBox(width: MinosSpacing.sm),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: <Widget>[
                            Text(
                              widget.toolName,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: theme.textTheme.titleSmall,
                            ),
                            const SizedBox(height: 2),
                            Text(
                              statusLabel,
                              style: theme.textTheme.labelSmall?.copyWith(
                                color: statusColor,
                              ),
                            ),
                          ],
                        ),
                      ),
                      Icon(
                        _expanded
                            ? CupertinoIcons.chevron_up
                            : CupertinoIcons.chevron_down,
                        size: 14,
                        color: colors.textTertiary,
                      ),
                    ],
                  ),
                ),
              ),
            ),
            if (_expanded)
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  MinosSpacing.md,
                  0,
                  MinosSpacing.md,
                  MinosSpacing.md,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    _LabeledBlock(
                      label: 'args',
                      child: HighlightView(
                        _prettyJson(widget.argsJson),
                        language: 'json',
                        theme: colors.isDark
                            ? atomOneDarkTheme
                            : atomOneLightTheme,
                        padding: const EdgeInsets.all(MinosSpacing.sm),
                        textStyle: const TextStyle(
                          fontFamily: 'monospace',
                          fontSize: 12,
                        ),
                      ),
                    ),
                    if (widget.output != null) ...<Widget>[
                      const SizedBox(height: MinosSpacing.sm),
                      _LabeledBlock(
                        label: widget.isError ? 'error' : 'output',
                        child: SelectableText(
                          widget.output!,
                          style: theme.textTheme.bodySmall?.copyWith(
                            fontFamily: 'monospace',
                            color: widget.isError ? colors.danger : null,
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
              ),
          ],
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
    final colors = context.minosColors;
    if (inFlight) {
      return SizedBox(
        width: 18,
        height: 18,
        child: CircularProgressIndicator(strokeWidth: 2, color: colors.accent),
      );
    }
    if (isError) {
      return Icon(
        CupertinoIcons.xmark_circle_fill,
        size: 18,
        color: colors.danger,
      );
    }
    return Icon(
      CupertinoIcons.checkmark_circle_fill,
      size: 18,
      color: colors.success,
    );
  }
}

class _LabeledBlock extends StatelessWidget {
  const _LabeledBlock({required this.label, required this.child});
  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.minosColors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        Text(
          label,
          style: theme.textTheme.labelSmall?.copyWith(
            color: colors.textTertiary,
          ),
        ),
        const SizedBox(height: MinosSpacing.xs),
        Container(
          decoration: BoxDecoration(
            color: colors.surfaceMuted,
            borderRadius: MinosRadii.xsAll,
          ),
          padding: const EdgeInsets.all(MinosSpacing.xs),
          child: child,
        ),
      ],
    );
  }
}
