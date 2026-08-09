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
   * @deprecated Prefer hubMinLoadedSeq / hostMinLoadedSeq. Kept as
   * hubMinLoadedSeq when linked, hostMinLoadedSeq for local-only.
   */
  firstLoadedSeq: number | null;
  /** Lowest Hub social message_seq in the window (Hub before_seq). */
  hubMinLoadedSeq: number | null;
  /** Highest Hub social message_seq in the window. */
  hubMaxLoadedSeq: number | null;
  /** Lowest host daemon seq among host-only cards / local-only chat. */
  hostMinLoadedSeq: number | null;
  /**
   * Lowest Hub `created_at_ms` in the window (display/debug only).
   */
  firstLoadedCreatedAtMs?: number | null;
  hasOlderHub: boolean;
  hasOlderHost: boolean;
  /** True when either Hub or host has older history. */
  hasOlder: boolean;
  /** Quiet older-page fetch in flight. */
  loadingOlder: boolean;
};

export const EMPTY_MESSAGE_HISTORY: MessageHistoryMeta = Object.freeze({
  firstLoadedSeq: null,
  hubMinLoadedSeq: null,
  hubMaxLoadedSeq: null,
  hostMinLoadedSeq: null,
  firstLoadedCreatedAtMs: null,
  hasOlderHub: false,
  hasOlderHost: false,
  hasOlder: false,
  loadingOlder: false,
});

export function emptyMessageHistoryMeta(): MessageHistoryMeta {
  return { ...EMPTY_MESSAGE_HISTORY };
}

/** Build meta from a merged window + independent has-more flags. */
export function messageHistoryFromWindow(
  messages: readonly TimelineMessage[],
  opts: {
    hasOlderHub?: boolean;
    hasOlderHost?: boolean;
    loadingOlder?: boolean;
    firstLoadedCreatedAtMs?: number | null;
    prev?: MessageHistoryMeta;
  } = {},
): MessageHistoryMeta {
  const hubMin = firstMessageSeq(messages);
  const hubMax = lastMessageSeq(messages);
  const hostMin = firstHostMessageSeq(messages);
  const hasOlderHub = opts.hasOlderHub ?? opts.prev?.hasOlderHub ?? false;
  const hasOlderHost = opts.hasOlderHost ?? opts.prev?.hasOlderHost ?? false;
  return {
    firstLoadedSeq: hubMin ?? hostMin,
    hubMinLoadedSeq: hubMin,
    hubMaxLoadedSeq: hubMax,
    hostMinLoadedSeq: hostMin,
    firstLoadedCreatedAtMs:
      opts.firstLoadedCreatedAtMs ??
      firstMessageCreatedAtMs(messages) ??
      opts.prev?.firstLoadedCreatedAtMs ??
      null,
    hasOlderHub,
    hasOlderHost,
    hasOlder: hasOlderHub || hasOlderHost,
    loadingOlder: opts.loadingOlder ?? false,
  };
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

/** Lowest social `messageSeq` (Hub or pure-local chat), ignoring host cards. */
export function firstMessageSeq(
  messages: readonly TimelineMessage[],
): number | null {
  let min: number | null = null;
  for (const m of messages) {
    if (m.kind === "tool_summary" || m.kind === "git_activity" || m.kind === "approval") {
      continue;
    }
    if (m.kind === "system" && m.role === "system") continue;
    const seq = m.messageSeq;
    if (seq == null) continue;
    if (min == null || seq < min) min = seq;
  }
  return min;
}

/** Highest social `messageSeq` in a list, or null if none. */
export function lastMessageSeq(
  messages: readonly TimelineMessage[],
): number | null {
  let max: number | null = null;
  for (const m of messages) {
    if (m.kind === "tool_summary" || m.kind === "git_activity" || m.kind === "approval") {
      continue;
    }
    if (m.kind === "system" && m.role === "system") continue;
    const seq = m.messageSeq;
    if (seq == null) continue;
    if (max == null || seq > max) max = seq;
  }
  return max;
}

/** Lowest host daemon seq among host-only cards (or hostMessageSeq). */
export function firstHostMessageSeq(
  messages: readonly TimelineMessage[],
): number | null {
  let min: number | null = null;
  for (const m of messages) {
    const seq = m.hostMessageSeq ?? (
      m.kind === "tool_summary" ||
      m.kind === "git_activity" ||
      m.kind === "approval" ||
      (m.kind === "system" && m.role === "system")
        ? m.messageSeq
        : undefined
    );
    if (seq == null || !Number.isFinite(seq)) continue;
    if (min == null || seq < min) min = seq;
  }
  return min;
}

/**
 * Meta after a tail (or full open) page.
 * `hasMoreHost` / `hasMoreHub` are independent namespace flags.
 */
export function metaAfterMessageTail(
  messages: readonly TimelineMessage[],
  hasMore: boolean,
  firstLoadedCreatedAtMs?: number | null,
  opts?: { hasMoreHub?: boolean; hasMoreHost?: boolean },
): MessageHistoryMeta {
  const hubMin = firstMessageSeq(messages);
  const hubMax = lastMessageSeq(messages);
  const hostMin = firstHostMessageSeq(messages);
  const hasOlderHub = opts?.hasMoreHub ?? hasMore;
  const hasOlderHost = opts?.hasMoreHost ?? hasMore;
  return {
    firstLoadedSeq: hubMin ?? hostMin,
    hubMinLoadedSeq: hubMin,
    hubMaxLoadedSeq: hubMax,
    hostMinLoadedSeq: hostMin,
    firstLoadedCreatedAtMs: firstLoadedCreatedAtMs ?? null,
    hasOlderHub,
    hasOlderHost,
    hasOlder: hasOlderHub || hasOlderHost,
    loadingOlder: false,
  };
}

/** Lowest createdAtMs in a list (Hub before_ts_ms cursor). */
export function firstMessageCreatedAtMs(
  messages: readonly TimelineMessage[],
): number | null {
  let min: number | null = null;
  for (const m of messages) {
    const ts = m.createdAtMs;
    if (ts == null || !Number.isFinite(ts)) continue;
    if (min == null || ts < min) min = ts;
  }
  return min;
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
