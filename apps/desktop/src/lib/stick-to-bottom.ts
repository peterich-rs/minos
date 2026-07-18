/**
 * Stick-to-bottom / follow semantics aligned with TUI ChatState:
 * - following → always pin to max scroll
 * - scroll away from bottom → unfollow
 * - return near bottom → re-follow
 * - while not following, never programmatically move the viewport
 */

export const FOLLOW_THRESHOLD_PX = 80;

export function distanceFromBottom(el: {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight;
}

export function isNearBottom(
  distance: number,
  threshold: number = FOLLOW_THRESHOLD_PX,
): boolean {
  return distance <= threshold;
}

/** After a user scroll event, should we be following? */
export function followAfterUserScroll(
  distance: number,
  threshold: number = FOLLOW_THRESHOLD_PX,
): boolean {
  return isNearBottom(distance, threshold);
}

export type FollowContentItem = {
  id: string;
  seq?: number;
  kind?: string;
  text?: string;
  detail?: string | null;
};

/**
 * Stable key that changes when list length, last identity, or last body grows
 * (covers streaming in-place merges that keep `items.length` constant).
 */
export function followContentKey(items: FollowContentItem[]): string {
  const last = items[items.length - 1];
  if (!last) return "0";
  return [
    items.length,
    last.id,
    last.seq ?? 0,
    last.kind ?? "",
    last.text?.length ?? 0,
    last.detail?.length ?? 0,
  ].join(":");
}
