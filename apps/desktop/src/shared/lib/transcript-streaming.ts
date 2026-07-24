/**
 * Session-transcript streaming cursor policy (desktop).
 *
 * TUI ChatState and mobile thread_event_timeline clear text/reasoning
 * streaming when tools start (`finish_open_content_streaming` /
 * `closeTextSegment`). Desktop must match: only the **timeline tail** may
 * show the pulse caret — never the last assistant bubble while tools or
 * subagent status rows sit after it (session can stay `running` for a long
 * time after intermediate narration ends).
 */

/**
 * Kinds that may render a live streaming caret.
 *
 * User rows are intentionally excluded: right after send the tail is often the
 * user bubble while `session.status === "running"`, and a caret there looks
 * like the agent is still typing into the user message.
 */
export const STREAMABLE_TRANSCRIPT_KINDS = new Set([
  "assistant",
  "text",
  "reasoning",
]);

export function isStreamableTranscriptKind(kind: string): boolean {
  return STREAMABLE_TRANSCRIPT_KINDS.has(kind);
}

/**
 * Id of the item that may show a streaming cursor, or null.
 *
 * Returns null when the tail is a tool / status / approval / etc. — even if
 * an earlier assistant/text/reasoning row is still the last "streamable"
 * kind (that old walk-back heuristic caused leftover █ cursors).
 */
export function streamingTailItemId(
  items: ReadonlyArray<{ id: string; kind: string }>,
): string | null {
  const last = items[items.length - 1];
  if (!last || !isStreamableTranscriptKind(last.kind)) return null;
  return last.id;
}

/** Whether this row should paint the streaming caret. */
export function itemShowsStreamingCursor(
  item: { id: string; kind: string },
  opts: {
    sessionLive: boolean;
    streamingTailId: string | null;
  },
): boolean {
  if (!opts.sessionLive || opts.streamingTailId == null) return false;
  if (item.id !== opts.streamingTailId) return false;
  return isStreamableTranscriptKind(item.kind);
}
