import type { TimelineMessage } from "../../../shared/lib/mock-data.ts";
import type { ConversationReadCursor } from "../../read-state/lib/read-state.ts";
import { firstUnreadMessageIndex } from "../../read-state/lib/read-state.ts";
import {
  isMessageGroupContinuation,
  shouldShowDayDivider,
} from "./message-grouping.ts";

export type VirtualTimelineItem =
  | { type: "day"; id: string; ms: number }
  | { type: "unread"; id: string }
  | {
      type: "message";
      id: string;
      message: TimelineMessage;
      groupedWithPrevious: boolean;
    };

export type BuildVirtualTimelineOptions = {
  /** Client read frontier; inserts an unread divider when mid-window. */
  readCursor?: ConversationReadCursor;
};

/**
 * Flatten sorted timeline messages into virtualizer rows
 * (day dividers + optional unread divider + messages).
 */
export function buildVirtualTimelineItems(
  messages: TimelineMessage[],
  options: BuildVirtualTimelineOptions = {},
): VirtualTimelineItem[] {
  const unreadAt = firstUnreadMessageIndex(messages, options.readCursor);
  const out: VirtualTimelineItem[] = [];
  for (let i = 0; i < messages.length; i++) {
    if (i === unreadAt) {
      out.push({ type: "unread", id: "unread-divider" });
    }
    const message = messages[i]!;
    const prev = i > 0 ? messages[i - 1] : undefined;
    if (shouldShowDayDivider(prev, message) && message.createdAtMs) {
      out.push({
        type: "day",
        id: `day-${message.id}`,
        ms: message.createdAtMs,
      });
    }
    // Break visual grouping across the unread divider.
    const groupedWithPrevious =
      i !== unreadAt && isMessageGroupContinuation(prev, message);
    out.push({
      type: "message",
      id: message.id,
      message,
      groupedWithPrevious,
    });
  }
  return out;
}

/** Estimate row height before measure (cheap). */
export function estimateVirtualTimelineItemSize(
  item: VirtualTimelineItem,
): number {
  if (item.type === "day") return 36;
  if (item.type === "unread") return 40;
  const bodyLen = item.message.body?.length ?? 0;
  const lines = Math.min(12, Math.max(1, Math.ceil(bodyLen / 80)));
  const base = item.groupedWithPrevious ? 28 : 52;
  return base + lines * 18;
}
