import 'package:flutter/material.dart';

/// Renders a horizontal row of workspace MRU chips.
/// Tapping a chip calls [onSelect] with the workspace path.
class WorkspaceMruChips extends StatelessWidget {
  const WorkspaceMruChips({
    super.key,
    required this.entries,
    required this.onSelect,
  });

  final List<String> entries;
  final ValueChanged<String> onSelect;

  @override
  Widget build(BuildContext context) {
    if (entries.isEmpty) return const SizedBox.shrink();

    return SizedBox(
      height: 36,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 16),
        itemCount: entries.length,
        separatorBuilder: (_, _) => const SizedBox(width: 8),
        itemBuilder: (context, index) {
          final workspace = entries[index];
          return ActionChip(
            label: Text(
              workspace,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.labelSmall,
            ),
            onPressed: () => onSelect(workspace),
            visualDensity: VisualDensity.compact,
          );
        },
      ),
    );
  }
}
