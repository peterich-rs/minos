import type { TimelineMessage } from "./mock-data.ts";
import { sortTimelineMessages } from "./timeline-order.ts";
import { timelineMessageEqual } from "./list-identity.ts";

/** Default page size for conversation timeline (tail + older). */
export const MESSAGE_PAGE_SIZE = 80;

/**
 * Hard cap on messages retained per conversation timeline window (spec §21).
 * Pin/focus may still trim oldest; set hasOlder when trimmed.
 */
export const TIMELINE_HARD_MAX_MESSAGES = 500;

/** Autofill slack when opening a short tail that does not fill the viewport. */
export const MESSAGE_AUTOFILL_SLACK_PX = 96;

export type MessageHistoryMeta = {
  /**
   * Lowest durable `messageSeq` currently loaded (inclusive).
   * Used as `before_seq` for the next older page. Null when empty/unknown.
   */
  firstLoadedSeq: number | null;
  /** True when the last older/tail fetch reported more history above. */
  hasOlder: boolean;
  /** Quiet older-page fetch in flight. */
  loadingOlder: boolean;
};

export const EMPTY_MESSAGE_HISTORY: MessageHistoryMeta = Object.freeze({
  firstLoadedSeq: null,
  hasOlder: false,
  loadingOlder: false,
});

export function emptyMessageHistoryMeta(): MessageHistoryMeta {
  return EMPTY_MESSAGE_HISTORY;
}

/**
 * Whether live conversation events may quiet re-list messages for `conversationId`.
 *
 * True when a Timeline working-set **key already exists** (messages map, history
 * meta, or timeline status — including empty `[]` after ensureLoaded started).
 * LiveIngress must not create Timeline windows for background conversations.
 */
export function hasTimelineWorkingSet(
  messagesByConversation: Record<string, unknown>,
  conversationId: string,
  extras?: {
    messageHistoryByConversation?: Record<string, unknown>;
    timelineStatusByConversation?: Record<string, unknown>;
  },
): boolean {
  if (
    Object.prototype.hasOwnProperty.call(messagesByConversation, conversationId)
  ) {
    return true;
  }
  if (
    extras?.messageHistoryByConversation &&
    Object.prototype.hasOwnProperty.call(
      extras.messageHistoryByConversation,
      conversationId,
    )
  ) {
    return true;
  }
  if (
    extras?.timelineStatusByConversation &&
    Object.prototype.hasOwnProperty.call(
      extras.timelineStatusByConversation,
      conversationId,
    )
  ) {
    return true;
  }
  return false;
}

/** Lowest durable seq in a list, or null if none. */
export function firstMessageSeq(
  messages: readonly TimelineMessage[],
): number | null {
  let min: number | null = null;
  for (const m of messages) {
    const seq = m.messageSeq;
    if (seq == null) continue;
    if (min == null || seq < min) min = seq;
  }
  return min;
}

/**
 * Meta after a tail (or full open) page: `hasOlder` from the daemon flag.
 */
export function metaAfterMessageTail(
  messages: readonly TimelineMessage[],
  hasMore: boolean,
): MessageHistoryMeta {
  return {
    firstLoadedSeq: firstMessageSeq(messages),
    hasOlder: hasMore,
    loadingOlder: false,
  };
}

/**
 * Merge an older page (ASC) in front of the already-loaded newer window.
 * Dedupes by id; reuses stable row identity when content matches.
 */
export function mergeMessagesOlder(
  older: TimelineMessage[],
  newer: TimelineMessage[],
): TimelineMessage[] {
  if (older.length === 0) return newer;
  if (newer.length === 0) return older;

  const newerIds = new Set(newer.map((m) => m.id));
  const olderUnique = older.filter((m) => !newerIds.has(m.id));
  if (olderUnique.length === 0) return newer;

  // Prefer previous object identity for rows that already exist in `newer`.
  return sortTimelineMessages([...olderUnique, ...newer]);
}

/**
 * Quiet re-list merge: keep previously loaded older pages, upsert the tail
 * page by id, drop pending rows that the server has replaced.
 *
 * Without this, a quiet tail re-list of N messages would wipe older pages.
 */
export function mergeMessagesQuietTail(
  prev: TimelineMessage[] | undefined,
  tail: TimelineMessage[],
): TimelineMessage[] {
  if (!prev || prev.length === 0) return sortTimelineMessages(tail);

  const byId = new Map<string, TimelineMessage>();
  for (const m of prev) byId.set(m.id, m);

  for (const incoming of tail) {
    const existing = byId.get(incoming.id);
    if (existing && timelineMessageEqual(existing, incoming)) {
      byId.set(incoming.id, existing);
    } else {
      byId.set(incoming.id, incoming);
    }
  }

  // Drop optimistic pending rows once a durable message with same body appears
  // is handled by sendMessage replace paths; keep remaining pending here.
  return sortTimelineMessages([...byId.values()]);
}

/**
 * Drop oldest messages when over hardMax (ASC window → keep newest tail).
 * Caller should set hasOlder=true when trimmed.
 */
export function trimMessagesHardMax(
  messages: readonly TimelineMessage[],
  hardMax: number = TIMELINE_HARD_MAX_MESSAGES,
): { messages: TimelineMessage[]; trimmed: boolean } {
  if (hardMax <= 0 || messages.length <= hardMax) {
    return { messages: messages as TimelineMessage[], trimmed: false };
  }
  return {
    messages: messages.slice(messages.length - hardMax) as TimelineMessage[],
    trimmed: true,
  };
}
