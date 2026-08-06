import type { TimelineMessage } from "@/shared/lib/mock-data";

/**
 * Canonical conversation timeline order (C2 final).
 *
 * Cross-source durable rows: order **only** by `messageSeq` ASC when both
 * have seq. Never coerce missing seq to 0.
 *
 * Rows without `messageSeq` are only allowed for **local optimistic** sends
 * (`sending` / `failed`); they sort after durable seq peers (window tail),
 * then by `createdAtMs` / id among themselves.
 *
 * Mixed durable (has seq) vs durable-without-seq: prefer seq-bearing first by
 * createdAt only among no-seq peers — do not invent pseudo-seq.
 */
export function sortTimelineMessages(
  messages: TimelineMessage[],
): TimelineMessage[] {
  return [...messages].sort((a, b) => {
    const sa = a.messageSeq;
    const sb = b.messageSeq;
    const aHas = sa != null && Number.isFinite(sa);
    const bHas = sb != null && Number.isFinite(sb);

    if (aHas && bHas && sa !== sb) {
      return (sa as number) - (sb as number);
    }

    const aOpt =
      a.deliveryStatus === "sending" || a.deliveryStatus === "failed";
    const bOpt =
      b.deliveryStatus === "sending" || b.deliveryStatus === "failed";

    // Optimistic without seq: after durable peers.
    if (aOpt && !bOpt && !aHas) return 1;
    if (bOpt && !aOpt && !bHas) return -1;

    // One side has seq, the other is optimistic/no-seq: seq first.
    if (aHas && !bHas) return -1;
    if (bHas && !aHas) return 1;

    const ta = a.createdAtMs ?? 0;
    const tb = b.createdAtMs ?? 0;
    if (ta !== tb) {
      return ta - tb;
    }

    const aSending = a.deliveryStatus === "sending" ? 1 : 0;
    const bSending = b.deliveryStatus === "sending" ? 1 : 0;
    if (aSending !== bSending) {
      return aSending - bSending;
    }

    return a.id.localeCompare(b.id);
  });
}
