/**
 * Client-side conversation read-state (viewer progress).
 *
 * Lives on the desktop consumption side — not in daemon. Daemon owns host facts
 * (messages, sessions); this module tracks "where did *this* client leave off".
 *
 * Unread is primarily count-based (`messageCount - readMessageCount`) so list
 * hydrate stays cheap without loading every timeline. Optional
 * `lastReadMessageId` / `lastReadSeq` anchor the unread divider when the
 * timeline is open.
 */

export type ConversationReadCursor = {
  /** messageCount when the user last marked this conversation read. */
  readMessageCount: number;
  /** Optional durable message id at the read frontier (for unread divider). */
  lastReadMessageId?: string;
  /** Optional message_seq at the read frontier. */
  lastReadSeq?: number;
  updatedAtMs: number;
};

export type ReadCursorMap = Record<string, ConversationReadCursor>;

/** Legacy persist shape (workspace-store v1). */
export type LegacyReadMessageCountMap = Record<string, number>;

export function isConversationReadCursor(
  value: unknown,
): value is ConversationReadCursor {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return typeof v.readMessageCount === "number" && Number.isFinite(v.readMessageCount);
}

/**
 * Migrate legacy `readMessageCountById` map → cursor map.
 * Also accepts an already-migrated cursor map (pass-through).
 */
export function migrateReadCursors(
  raw: unknown,
  nowMs: number = Date.now(),
): ReadCursorMap {
  if (!raw || typeof raw !== "object") return {};
  const out: ReadCursorMap = {};
  for (const [id, value] of Object.entries(raw as Record<string, unknown>)) {
    if (!id) continue;
    if (typeof value === "number" && Number.isFinite(value)) {
      out[id] = {
        readMessageCount: Math.max(0, Math.floor(value)),
        updatedAtMs: nowMs,
      };
      continue;
    }
    if (isConversationReadCursor(value)) {
      out[id] = {
        readMessageCount: Math.max(0, Math.floor(value.readMessageCount)),
        lastReadMessageId: value.lastReadMessageId,
        lastReadSeq:
          typeof value.lastReadSeq === "number" && Number.isFinite(value.lastReadSeq)
            ? value.lastReadSeq
            : undefined,
        updatedAtMs:
          typeof value.updatedAtMs === "number" && Number.isFinite(value.updatedAtMs)
            ? value.updatedAtMs
            : nowMs,
      };
    }
  }
  return out;
}

/** Count-based unread; focused conversation is always 0. */
export function unreadCountFromCursor(
  messageCount: number,
  cursor: ConversationReadCursor | undefined,
  isFocused: boolean,
): number {
  if (isFocused) return 0;
  if (!cursor) return 0;
  return Math.max(0, messageCount - cursor.readMessageCount);
}

export type AdvanceReadCursorInput = {
  messageCount: number;
  lastReadMessageId?: string;
  lastReadSeq?: number;
  nowMs?: number;
};

/**
 * Advance (never rewind) a conversation read cursor.
 * Count and optional id/seq frontiers only move forward.
 */
export function advanceReadCursor(
  prev: ConversationReadCursor | undefined,
  input: AdvanceReadCursorInput,
): ConversationReadCursor {
  const nowMs = input.nowMs ?? Date.now();
  const nextCount = Math.max(
    prev?.readMessageCount ?? 0,
    Math.max(0, Math.floor(input.messageCount)),
  );
  let lastReadSeq = prev?.lastReadSeq;
  if (
    typeof input.lastReadSeq === "number" &&
    Number.isFinite(input.lastReadSeq)
  ) {
    lastReadSeq = Math.max(lastReadSeq ?? 0, input.lastReadSeq);
  }
  let lastReadMessageId = prev?.lastReadMessageId;
  if (input.lastReadMessageId) {
    // Prefer the newest seq's id when seq advances; otherwise keep prior id
    // unless we had none.
    if (
      lastReadSeq != null &&
      prev?.lastReadSeq != null &&
      lastReadSeq > prev.lastReadSeq
    ) {
      lastReadMessageId = input.lastReadMessageId;
    } else if (!lastReadMessageId || input.lastReadSeq == null) {
      lastReadMessageId = input.lastReadMessageId;
    } else if (input.lastReadSeq === lastReadSeq) {
      lastReadMessageId = input.lastReadMessageId;
    }
  }
  return {
    readMessageCount: nextCount,
    lastReadMessageId,
    lastReadSeq,
    updatedAtMs: nowMs,
  };
}

/**
 * Ensure first-sight conversations start as fully read (no historical flood).
 * Does not overwrite existing cursors.
 */
export function seedReadCursorIfAbsent(
  map: ReadCursorMap,
  conversationId: string,
  messageCount: number,
  nowMs: number = Date.now(),
): ReadCursorMap {
  if (map[conversationId] !== undefined) return map;
  return {
    ...map,
    [conversationId]: {
      readMessageCount: Math.max(0, Math.floor(messageCount)),
      updatedAtMs: nowMs,
    },
  };
}

/** Pick the latest message in a sorted-or-unsorted list for frontier stamp. */
export function latestMessageFrontier(
  messages: Array<{ id: string; messageSeq?: number }>,
): { lastReadMessageId?: string; lastReadSeq?: number } {
  if (messages.length === 0) return {};
  let best = messages[0]!;
  for (let i = 1; i < messages.length; i++) {
    const m = messages[i]!;
    const bestSeq = best.messageSeq ?? -1;
    const seq = m.messageSeq ?? -1;
    if (seq > bestSeq) best = m;
  }
  return {
    lastReadMessageId: best.id,
    lastReadSeq: best.messageSeq,
  };
}

/**
 * Index of the first message that is *after* the read frontier, or -1 if
 * everything is read / frontier unknown.
 *
 * Prefer seq match; fall back to message id.
 */
export function firstUnreadMessageIndex(
  messages: Array<{ id: string; messageSeq?: number }>,
  cursor: ConversationReadCursor | undefined,
): number {
  if (!cursor || messages.length === 0) return -1;
  if (cursor.lastReadSeq != null) {
    for (let i = 0; i < messages.length; i++) {
      const seq = messages[i]!.messageSeq;
      if (typeof seq === "number" && seq > cursor.lastReadSeq) return i;
    }
    // All loaded messages are at or before the frontier.
    return -1;
  }
  if (cursor.lastReadMessageId) {
    const idx = messages.findIndex((m) => m.id === cursor.lastReadMessageId);
    if (idx >= 0 && idx < messages.length - 1) return idx + 1;
    return -1;
  }
  return -1;
}
