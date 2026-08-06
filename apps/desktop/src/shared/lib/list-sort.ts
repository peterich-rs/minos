/** WeChat-style list ordering: attention first, then recency. */

export type AttentionSortable = {
  /** Project: hasUnread; conversation: (unread + approvalCount) > 0. */
  hasUnread: boolean;
  /**
   * Last activity epoch ms (sort SSOT within both attention and quiet groups).
   * There is no separate "attention bump" clock on Conversation yet — both
   * groups order by the same last-activity signal.
   */
  updatedAtMs: number;
};

/**
 * Sort key: rows with attention before rows without; within each group by
 * `updatedAtMs` DESC.
 */
export function sortByAttentionThenTime<T extends AttentionSortable>(
  a: T,
  b: T,
): number {
  const au = a.hasUnread ? 1 : 0;
  const bu = b.hasUnread ? 1 : 0;
  if (au !== bu) return bu - au;
  return b.updatedAtMs - a.updatedAtMs;
}
