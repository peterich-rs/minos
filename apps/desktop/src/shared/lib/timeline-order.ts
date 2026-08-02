import type { TimelineMessage } from "@/shared/lib/mock-data";

/**
 * Canonical conversation timeline order.
 *
 * 1. When **both** rows have `messageSeq`, order by seq ASC (daemon durable order).
 * 2. Otherwise fall back to `createdAtMs` ASC — **do not** put every seq-bearing
 *    row before every seq-less row. Hub-mapped bubbles often lack `messageSeq`;
 *    old behavior put local `agent-result` (has seq) above Hub/user rows (no seq)
 *    even when the user message was earlier.
 * 3. Optimistic `sending` rows sort after durable peers at the same clock.
 * 4. Stable tie-break: id.
 */
export function sortTimelineMessages(
  messages: TimelineMessage[],
): TimelineMessage[] {
  return [...messages].sort((a, b) => {
    const sa = a.messageSeq;
    const sb = b.messageSeq;
    if (sa != null && sb != null && sa !== sb) {
      return sa - sb;
    }

    const ta = a.createdAtMs ?? 0;
    const tb = b.createdAtMs ?? 0;
    if (ta !== tb) {
      return ta - tb;
    }

    // Same wall clock: prefer lower seq if only one side has it (user often
    // has seq from daemon; hub peer may not).
    if (sa != null && sb == null) return -1;
    if (sa == null && sb != null) return 1;

    const aSending = a.deliveryStatus === "sending" ? 1 : 0;
    const bSending = b.deliveryStatus === "sending" ? 1 : 0;
    if (aSending !== bSending) {
      return aSending - bSending;
    }

    return a.id.localeCompare(b.id);
  });
}
