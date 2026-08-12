/**
 * Per-topic durable resume cursors for Hub realtime.
 *
 * Keyed by full topic string (`account:{id}`, `conversation:{id}`).
 * Values are last applied `topic_seq` (resume_after = that seq).
 *
 * Pure helpers are unit-tested with node:test (no @/ path aliases).
 */

export type TopicCursorMap = Record<string, number>;

export const CLOUD_CURSOR_STORAGE_KEY = "minos.cloud.topic_cursors.v1";
/** Legacy storage key; read for migration, never written. */
export const LEGACY_CLOUD_CURSOR_STORAGE_KEY = "minos.hub.topic_cursors.v1";

function parseTopicCursors(raw: string | null): TopicCursorMap {
  if (!raw) return {};
  try {
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

/** Result of attempting to advance a topic cursor with continuity checks. */
export type AdvanceCursorResult =
  | { kind: "unchanged"; cursors: TopicCursorMap }
  | { kind: "advanced"; cursors: TopicCursorMap }
  | { kind: "hole"; cursors: TopicCursorMap; expected: number; got: number };

/**
 * Pure: true when `topicSeq` skips ahead of a known applied cursor.
 * Fresh/cleared cursors (missing or 0) never report a hole — SnapshotRequired
 * catch-up and first-subscribe may land on a high seq after retention.
 */
export function isTopicSeqHole(
  cursors: TopicCursorMap,
  topic: string,
  topicSeq: number,
): boolean {
  if (!topic || !Number.isFinite(topicSeq) || topicSeq < 0) return false;
  const prev = cursors[topic];
  if (prev == null || prev <= 0) return false;
  return topicSeq > prev + 1;
}

/** Pure: merge one topic_seq into a cursor map (monotonic, no hole check). */
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

/**
 * Pure: advance only on continuous seq (cursor+1 or first/zero cursor).
 * Holes leave the map unchanged so the caller can request SnapshotRequired.
 */
export function tryAdvanceTopicCursor(
  cursors: TopicCursorMap,
  topic: string,
  topicSeq: number,
): AdvanceCursorResult {
  if (!topic || !Number.isFinite(topicSeq) || topicSeq < 0) {
    return { kind: "unchanged", cursors };
  }
  const prev = cursors[topic];
  if (prev != null && topicSeq <= prev) {
    return { kind: "unchanged", cursors };
  }
  if (prev != null && prev > 0 && topicSeq > prev + 1) {
    return { kind: "hole", cursors, expected: prev + 1, got: topicSeq };
  }
  return {
    kind: "advanced",
    cursors: { ...cursors, [topic]: topicSeq },
  };
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

/**
 * Load cursors from localStorage (browser / Tauri webview).
 * Read-new-first; fall back to legacy hub key and migrate write-new.
 */
export function loadTopicCursors(
  storage:
    | (Pick<Storage, "getItem"> & Partial<Pick<Storage, "setItem" | "removeItem">>)
    | null
    | undefined = defaultStorage(),
): TopicCursorMap {
  if (!storage) return {};
  try {
    const current = parseTopicCursors(storage.getItem(CLOUD_CURSOR_STORAGE_KEY));
    if (Object.keys(current).length > 0) {
      return current;
    }
    const legacy = parseTopicCursors(
      storage.getItem(LEGACY_CLOUD_CURSOR_STORAGE_KEY),
    );
    if (Object.keys(legacy).length === 0) {
      return {};
    }
    // Migrate: write-new (and drop old when possible).
    try {
      storage.setItem?.(CLOUD_CURSOR_STORAGE_KEY, JSON.stringify(legacy));
      storage.removeItem?.(LEGACY_CLOUD_CURSOR_STORAGE_KEY);
    } catch {
      /* quota / private mode — still return migrated map in-memory */
    }
    return legacy;
  } catch {
    return {};
  }
}

/** Persist cursors to localStorage (new key only). */
export function saveTopicCursors(
  cursors: TopicCursorMap,
  storage:
    | (Pick<Storage, "setItem"> & Partial<Pick<Storage, "removeItem">>)
    | null
    | undefined = defaultStorage(),
): void {
  if (!storage) return;
  try {
    storage.setItem(CLOUD_CURSOR_STORAGE_KEY, JSON.stringify(cursors));
    try {
      storage.removeItem?.(LEGACY_CLOUD_CURSOR_STORAGE_KEY);
    } catch {
      /* ignore legacy cleanup failures */
    }
  } catch {
    /* quota / private mode */
  }
}

/** Drop all Hub topic cursors (account leave — never resume under another account). */
export function clearAllTopicCursors(
  storage:
    | (Pick<Storage, "removeItem"> & Partial<Pick<Storage, "setItem">>)
    | null
    | undefined = defaultStorage(),
): void {
  if (!storage) return;
  try {
    storage.removeItem?.(CLOUD_CURSOR_STORAGE_KEY);
    storage.removeItem?.(LEGACY_CLOUD_CURSOR_STORAGE_KEY);
  } catch {
    try {
      storage.setItem?.(CLOUD_CURSOR_STORAGE_KEY, "{}");
    } catch {
      /* quota / private mode */
    }
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
