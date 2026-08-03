/**
 * Pure LRU for conversation topic subscriptions (Realtime Surface R4).
 * Kept in a path-alias-free module so node:test can import it directly.
 */

/** Max concurrent conversation topic subscriptions. */
export const MAX_OPEN_CONVERSATION_SUBSCRIPTIONS = 16;

/**
 * Touch `id` as most-recent. Returns next ordered list and ids to unsubscribe.
 */
export function conversationSubscriptionLruTouch(
  orderedIds: string[],
  id: string,
  maxOpen: number = MAX_OPEN_CONVERSATION_SUBSCRIPTIONS,
): { next: string[]; evicted: string[] } {
  const trimmed = id.trim();
  if (!trimmed) return { next: [...orderedIds], evicted: [] };
  const without = orderedIds.filter((x) => x !== trimmed);
  const next = [...without, trimmed];
  const evicted: string[] = [];
  while (next.length > maxOpen) {
    const old = next.shift();
    if (old) evicted.push(old);
  }
  return { next, evicted };
}
