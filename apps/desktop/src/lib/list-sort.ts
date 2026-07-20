/** WeChat-style list ordering: attention first, then recency. */

export type AttentionSortable = {
  /** Project: hasUnread; conversation: (unread + approvalCount) > 0. */
  hasUnread: boolean;
  /** Max attention-bump time among rows that currently need attention. */
  lastAttentionMs: number;
  updatedAtMs: number;
};

/**
 * Sort key: rows with attention before rows without;
 * within attention group by lastAttentionMs DESC;
 * within quiet group by updatedAtMs DESC.
 */
export function sortByAttentionThenTime<T extends AttentionSortable>(
  a: T,
  b: T,
): number {
  const au = a.hasUnread ? 1 : 0;
  const bu = b.hasUnread ? 1 : 0;
  if (au !== bu) return bu - au;
  if (au === 1) return b.lastAttentionMs - a.lastAttentionMs;
  return b.updatedAtMs - a.updatedAtMs;
}
