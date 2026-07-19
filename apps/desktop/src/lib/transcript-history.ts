import type { TranscriptItem } from "./daemon";

/** Default raw-event window for first open (tail) and each older page. */
export const TRANSCRIPT_PAGE_EVENTS = 400;

/** Prefetch older history when within this many px of the top. */
export const TRANSCRIPT_PREFETCH_TOP_PX = 720;

/** If content does not fill the viewport, keep backfilling (silent). */
export const TRANSCRIPT_AUTOFILL_SLACK_PX = 96;

export type TranscriptHistoryMeta = {
  /**
   * Lowest raw event seq (inclusive) covered by any load for this thread.
   * When `1`, there is nothing older to fetch.
   */
  firstLoadedStartSeq: number;
  /** True when firstLoadedStartSeq > 1. */
  hasOlder: boolean;
  /** Quiet older-page fetch in flight. */
  loadingOlder: boolean;
};

/** Stable empty meta for Zustand selectors (never allocate per getSnapshot). */
export const EMPTY_TRANSCRIPT_HISTORY: TranscriptHistoryMeta = Object.freeze({
  firstLoadedStartSeq: 1,
  hasOlder: false,
  loadingOlder: false,
});

export function emptyTranscriptHistoryMeta(): TranscriptHistoryMeta {
  return EMPTY_TRANSCRIPT_HISTORY;
}

/**
 * Compute exclusive `from_seq` for a tail window ending at `lastSeq`.
 * Returns undefined when the whole history fits in the window (start at 1).
 */
export function tailFromSeq(
  lastSeq: number,
  window: number = TRANSCRIPT_PAGE_EVENTS,
): number | undefined {
  if (lastSeq <= window) return undefined;
  // start = lastSeq - window + 1 → exclusive from = start - 1
  return lastSeq - window;
}

/**
 * Range for one older page immediately before `firstLoadedStartSeq`.
 * `fromSeq` is exclusive for the daemon; `limit` covers up to the previous event.
 */
export function olderPageRange(
  firstLoadedStartSeq: number,
  window: number = TRANSCRIPT_PAGE_EVENTS,
): { fromSeq: number; limit: number; nextFirstLoadedStartSeq: number } | null {
  if (firstLoadedStartSeq <= 1) return null;
  const end = firstLoadedStartSeq - 1; // inclusive last seq to fetch
  const start = Math.max(1, end - window + 1);
  return {
    fromSeq: start - 1,
    limit: end - start + 1,
    nextFirstLoadedStartSeq: start,
  };
}

const MERGEABLE_KINDS = new Set([
  "assistant",
  "text",
  "reasoning",
  "user",
]);

/**
 * Prepend an older assembled page onto the already-loaded newer items.
 * Merges boundary chunks that share messageId+kind (split by page assembly).
 */
export function mergeTranscriptOlder(
  older: TranscriptItem[],
  newer: TranscriptItem[],
): TranscriptItem[] {
  if (older.length === 0) return newer;
  if (newer.length === 0) return older;

  const newerIds = new Set(newer.map((it) => it.id));
  const olderUnique = older.filter((it) => !newerIds.has(it.id));
  if (olderUnique.length === 0) return newer;

  const last = olderUnique[olderUnique.length - 1]!;
  const first = newer[0]!;
  if (
    last.messageId &&
    first.messageId &&
    last.messageId === first.messageId &&
    last.kind === first.kind &&
    MERGEABLE_KINDS.has(last.kind)
  ) {
    const merged: TranscriptItem = {
      ...last,
      text: `${last.text}${first.text}`,
      detail: first.detail ?? last.detail,
      seq: Math.max(last.seq, first.seq),
      tsMs: Math.max(last.tsMs, first.tsMs),
    };
    return [...olderUnique.slice(0, -1), merged, ...newer.slice(1)];
  }

  return [...olderUnique, ...newer];
}

/** After a tail/full open: where does history start, and is there more above? */
export function metaAfterTailLoad(fromSeq: number | undefined): TranscriptHistoryMeta {
  const firstLoadedStartSeq = fromSeq === undefined ? 1 : fromSeq + 1;
  return {
    firstLoadedStartSeq,
    hasOlder: firstLoadedStartSeq > 1,
    loadingOlder: false,
  };
}
