/**
 * Conversation rail last-activity helpers (digest + list clock).
 *
 * `lastMessageAtMs` means "activity time of the current visible latest chat
 * bubble", not "any frame's createdAtMs" (recall must recompute, not reuse
 * the recalled row's original timestamp).
 */

import type { TimelineMessage } from "./mock-data.ts";

export type RailActivity = {
  lastMessageAtMs: number;
  preview: string;
};

/**
 * Walk timeline ASC tail → newest chat-like bubble for rail preview/clock.
 * Skips tool/git/system cards and empty bodies.
 */
export function railActivityFromTimeline(
  messages: readonly TimelineMessage[] | undefined | null,
): RailActivity | null {
  if (!messages?.length) return null;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]!;
    if (m.role === "system") continue;
    const kind = m.kind ?? "text";
    if (kind === "tool_summary" || kind === "git_activity" || kind === "approval") {
      continue;
    }
    const body = m.body?.trim() ?? "";
    if (!body) continue;
    const ms =
      m.createdAtMs != null && Number.isFinite(m.createdAtMs) && m.createdAtMs > 0
        ? m.createdAtMs
        : 0;
    return {
      lastMessageAtMs: ms,
      preview: body.length > 88 ? `${body.slice(0, 88)}…` : body,
    };
  }
  return null;
}

/**
 * Resolve digest last activity after a live frame.
 * - Append: monotonic max(incoming, previous); never invent Date.now().
 * - Recall: recompute from open timeline when present; otherwise keep previous
 *   (never apply the recalled message's original createdAtMs).
 */
export function resolveDigestLastActivityMs(input: {
  isRecall: boolean;
  /** Append path only: frame activity ms (0 = unknown / omit). */
  incomingLastAtMs: number;
  previousLastMessageAtMs: number;
  timeline?: readonly TimelineMessage[] | null;
}): number {
  const prev =
    Number.isFinite(input.previousLastMessageAtMs) &&
    input.previousLastMessageAtMs > 0
      ? input.previousLastMessageAtMs
      : 0;

  if (input.isRecall) {
    const fromWindow = railActivityFromTimeline(input.timeline);
    if (fromWindow && fromWindow.lastMessageAtMs > 0) {
      return fromWindow.lastMessageAtMs;
    }
    // Window not loaded or empty after recall — keep prior digest clock.
    return prev;
  }

  const incoming =
    Number.isFinite(input.incomingLastAtMs) && input.incomingLastAtMs > 0
      ? input.incomingLastAtMs
      : 0;
  return Math.max(incoming, prev);
}

/** Positive finite ms only — 0 / NaN / negative are "missing", not epoch activity. */
export function positiveMs(ms: number | null | undefined): number {
  if (ms == null || !Number.isFinite(ms) || ms <= 0) return 0;
  return ms;
}
