import type { TimelineMessage } from "@/shared/lib/mock-data";

/** Default continuity window (Slack-like). */
export const MESSAGE_GROUP_WINDOW_MS = 10 * 60 * 1000;

/**
 * Author key for grouping: user messages share one key; agent messages group
 * by agent + session; system / tool_summary never group.
 */
export function messageAuthorKey(m: TimelineMessage): string | null {
  if (m.role === "system") return null;
  if (m.kind === "tool_summary") return null;
  if (m.role === "user") return "user";
  const agent = m.agent ?? "agent";
  const session = m.sessionId ?? "";
  return `agent:${agent}:${session}`;
}

/**
 * True when `curr` should hide avatar/header as a continuation of `prev`
 * (same author, within the time window).
 */
export function isMessageGroupContinuation(
  prev: TimelineMessage | undefined,
  curr: TimelineMessage,
  windowMs: number = MESSAGE_GROUP_WINDOW_MS,
): boolean {
  if (!prev) return false;
  const prevKey = messageAuthorKey(prev);
  const currKey = messageAuthorKey(curr);
  if (!prevKey || !currKey || prevKey !== currKey) return false;

  const prevMs = prev.createdAtMs;
  const currMs = curr.createdAtMs;
  // Without timestamps, still collapse consecutive same-author bubbles.
  if (
    prevMs == null ||
    currMs == null ||
    !Number.isFinite(prevMs) ||
    !Number.isFinite(currMs) ||
    prevMs <= 0 ||
    currMs <= 0
  ) {
    return true;
  }
  return Math.abs(currMs - prevMs) <= windowMs;
}

/** Local calendar day key `YYYY-MM-DD`, or null when timestamp missing. */
export function localDayKey(ms: number | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms <= 0) return null;
  const d = new Date(ms);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** Human day divider label from a local day key or ms. */
export function formatDayDividerLabel(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);

  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();

  if (sameDay(d, today)) return "Today";
  if (sameDay(d, yesterday)) return "Yesterday";
  return d.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year:
      d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
}

/**
 * Whether to insert a day divider before `curr` given the previous message.
 * Requires both messages to have valid createdAtMs and different local days.
 */
export function shouldShowDayDivider(
  prev: TimelineMessage | undefined,
  curr: TimelineMessage,
): boolean {
  const currKey = localDayKey(curr.createdAtMs);
  if (!currKey) return false;
  if (!prev) return true;
  const prevKey = localDayKey(prev.createdAtMs);
  if (!prevKey) return true;
  return prevKey !== currKey;
}
