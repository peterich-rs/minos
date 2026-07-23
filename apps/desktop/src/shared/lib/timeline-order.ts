import type { TimelineMessage } from "@/shared/lib/mock-data";

/**
 * Canonical conversation timeline order: durable `messageSeq` ASC.
 * Optimistic rows (no seq, e.g. `deliveryStatus: "sending"`) sort after durable.
 */
export function sortTimelineMessages(
  messages: TimelineMessage[],
): TimelineMessage[] {
  return [...messages].sort((a, b) => {
    const sa = a.messageSeq;
    const sb = b.messageSeq;
    if (sa != null && sb != null && sa !== sb) return sa - sb;
    if (sa != null && sb == null) return -1;
    if (sa == null && sb != null) return 1;
    const ta = a.createdAtMs ?? 0;
    const tb = b.createdAtMs ?? 0;
    if (ta !== tb) return ta - tb;
    return a.id.localeCompare(b.id);
  });
}
