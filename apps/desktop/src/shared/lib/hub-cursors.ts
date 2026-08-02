/**
 * Per-topic durable resume cursors for Hub realtime (Phase 4).
 *
 * Keyed by full topic string (`account:{id}`, `conversation:{id}`).
 * Values are last applied `topic_seq` (resume_after = that seq).
 *
 * Pure helpers are unit-tested with node:test (no @/ path aliases).
 */

export type TopicCursorMap = Record<string, number>;

export const HUB_CURSOR_STORAGE_KEY = "minos.hub.topic_cursors.v1";

/** Pure: merge one topic_seq into a cursor map (monotonic). */
export function advanceTopicCursor(
  cursors: TopicCursorMap,
  topic: string,
  topicSeq: number,
): TopicCursorMap {
  if (!topic || !Number.isFinite(topicSeq) || topicSeq < 0) {
    return cursors;
  }
  const prev = cursors[topic];
  if (prev != null && topicSeq <= prev) {
    return cursors;
  }
  return { ...cursors, [topic]: topicSeq };
}

/** Pure: drop a topic cursor (e.g. SnapshotRequired). */
export function clearTopicCursor(
  cursors: TopicCursorMap,
  topic: string,
): TopicCursorMap {
  if (!Object.prototype.hasOwnProperty.call(cursors, topic)) {
    return cursors;
  }
  const next = { ...cursors };
  delete next[topic];
  return next;
}

/** Pure: build resume_after map for Subscribe (omit empty). */
export function resumeAfterFromCursors(
  cursors: TopicCursorMap,
  topics: readonly string[],
): Record<string, number> | undefined {
  const out: Record<string, number> = {};
  for (const t of topics) {
    const seq = cursors[t];
    if (seq != null && seq > 0) {
      out[t] = seq;
    }
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

export function conversationTopic(conversationId: string): string {
  return `conversation:${conversationId}`;
}

export function accountTopic(accountId: string): string {
  return `account:${accountId}`;
}

/** Load cursors from localStorage (browser / Tauri webview). */
export function loadTopicCursors(
  storage: Pick<Storage, "getItem"> | null | undefined = defaultStorage(),
): TopicCursorMap {
  if (!storage) return {};
  try {
    const raw = storage.getItem(HUB_CURSOR_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    const out: TopicCursorMap = {};
    for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof k === "string" && typeof v === "number" && Number.isFinite(v)) {
        out[k] = v;
      }
    }
    return out;
  } catch {
    return {};
  }
}

/** Persist cursors to localStorage. */
export function saveTopicCursors(
  cursors: TopicCursorMap,
  storage: Pick<Storage, "setItem"> | null | undefined = defaultStorage(),
): void {
  if (!storage) return;
  try {
    storage.setItem(HUB_CURSOR_STORAGE_KEY, JSON.stringify(cursors));
  } catch {
    /* quota / private mode */
  }
}

function defaultStorage(): Storage | null {
  try {
    if (typeof localStorage === "undefined") return null;
    return localStorage;
  } catch {
    return null;
  }
}
