/**
 * Local-timezone display helpers.
 *
 * Epoch ms is the only time SSOT on the wire and in store. Never persist
 * pre-formatted relative strings — format at render (or pass `nowMs` in tests).
 */

/** 24h clock in the user's local timezone (`HH:mm`). */
export function formatLocalClock(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** Full local datetime for hover / `title` attributes. */
export function formatLocalDateTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  return new Date(ms).toLocaleString();
}

function startOfLocalDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/**
 * Conversation-list / board activity label (WeChat-style calendar buckets).
 *
 * - Today → local `HH:mm`
 * - Yesterday → `Yesterday`
 * - Within last 6 days → weekday short
 * - Same calendar year → `Mon D`
 * - Older → `Mon D, YYYY`
 */
export function formatListActivityTime(
  ms: number,
  nowMs: number = Date.now(),
): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  if (!Number.isFinite(nowMs)) nowMs = Date.now();

  const then = new Date(ms);
  const now = new Date(nowMs);
  const dayThen = startOfLocalDay(then);
  const dayNow = startOfLocalDay(now);
  const dayDiff = Math.round((dayNow - dayThen) / 86_400_000);

  if (dayDiff < 0) {
    // Clock skew / future: still show a local clock so the cell is not empty.
    return formatLocalClock(ms);
  }
  if (dayDiff === 0) {
    return formatLocalClock(ms);
  }
  if (dayDiff === 1) {
    return "Yesterday";
  }
  if (dayDiff < 7) {
    return then.toLocaleDateString(undefined, { weekday: "short" });
  }
  if (then.getFullYear() === now.getFullYear()) {
    return then.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  }
  return then.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/**
 * Compact age label for diagnostics / non-list surfaces.
 * Prefer {@link formatListActivityTime} for conversation rails.
 * Ages of a calendar day or more use the list calendar buckets.
 */
export function formatRelative(ms: number, nowMs: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  if (!Number.isFinite(nowMs)) nowMs = Date.now();
  const dayThen = startOfLocalDay(new Date(ms));
  const dayNow = startOfLocalDay(new Date(nowMs));
  if (dayThen < dayNow) {
    return formatListActivityTime(ms, nowMs);
  }
  const delta = Math.max(0, nowMs - ms);
  const secs = Math.floor(delta / 1000);
  if (secs < 60) return secs < 5 ? "now" : `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  return `${hours}h`;
}
