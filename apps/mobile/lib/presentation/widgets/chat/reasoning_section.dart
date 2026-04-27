import 'package:flutter/material.dart';

/// Collapsible card that hosts the assistant's chain-of-thought
/// (`UiEventMessage_ReasoningDelta` accumulation). Collapsed by default
/// — reasoning is auxiliary; the user opts in to see it. The body
/// renders monospace italic to differentiate from regular markdown.
class ReasoningSection extends StatelessWidget {
  const ReasoningSection({
    super.key,
    required this.messageId,
    required this.reasoningText,
    this.initiallyExpanded = false,
  });

  /// The assistant `message_id` whose reasoning deltas accumulate here.
  final String messageId;

  /// Concatenation of every `UiEventMessage_ReasoningDelta.text` in
  /// arrival order.
  final String reasoningText;

  /// Default expansion state. ThreadViewPage keeps reasoning collapsed
  /// so the chat surface isn't dominated by it.
  final bool initiallyExpanded;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Theme(
        // Strip the default ExpansionTile divider — it clashes with the
        // surrounding bubble layout.
        data: theme.copyWith(dividerColor: Colors.transparent),
        child: ExpansionTile(
          tilePadding: const EdgeInsets.symmetric(horizontal: 8),
          childrenPadding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
          initiallyExpanded: initiallyExpanded,
          leading: const Icon(Icons.psychology_outlined, size: 18),
          title: Text(
            'Reasoning',
            style: theme.textTheme.labelMedium?.copyWith(
              fontStyle: FontStyle.italic,
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          children: [
            Align(
              alignment: Alignment.centerLeft,
              child: SelectableText(
                reasoningText.isEmpty ? '(empty)' : reasoningText,
                style: theme.textTheme.bodySmall?.copyWith(
                  fontFamily: 'monospace',
                  fontStyle: FontStyle.italic,
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
