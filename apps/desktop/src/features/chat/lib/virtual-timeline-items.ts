import type { TimelineMessage } from "@/shared/domain/collaboration";
import {
  isMessageGroupContinuation,
  shouldShowDayDivider,
} from "./message-grouping.ts";

export type VirtualTimelineItem =
  | { type: "day"; id: string; ms: number }
  | {
      type: "message";
      id: string;
      message: TimelineMessage;
      groupedWithPrevious: boolean;
    };

/**
 * Flatten sorted timeline messages into virtualizer rows (day dividers + messages).
 */
export function buildVirtualTimelineItems(
  messages: TimelineMessage[],
): VirtualTimelineItem[] {
  const out: VirtualTimelineItem[] = [];
  for (let i = 0; i < messages.length; i++) {
    const message = messages[i]!;
    const prev = i > 0 ? messages[i - 1] : undefined;
    if (shouldShowDayDivider(prev, message) && message.createdAtMs) {
      out.push({
        type: "day",
        id: `day-${message.id}`,
        ms: message.createdAtMs,
      });
    }
    out.push({
      type: "message",
      id: message.id,
      message,
      groupedWithPrevious: isMessageGroupContinuation(prev, message),
    });
  }
  return out;
}

/** Estimate row height before measure (cheap). */
export function estimateVirtualTimelineItemSize(
  item: VirtualTimelineItem,
): number {
  if (item.type === "day") return 36;
  const bodyLen = item.message.body?.length ?? 0;
  const lines = Math.min(12, Math.max(1, Math.ceil(bodyLen / 80)));
  const base = item.groupedWithPrevious ? 28 : 52;
  return base + lines * 18;
}
