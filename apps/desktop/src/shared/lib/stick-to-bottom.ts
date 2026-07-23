/**
 * Stick-to-bottom / follow semantics aligned with TUI ChatState:
 * - following → always pin to max scroll
 * - scroll away from bottom → unfollow
 * - return near bottom → re-follow
 * - while not following, never programmatically move the viewport
 *
 * Rubber-band / rebound is handled by a short *suppress window* after wheel-up
 * (see UNFOLLOW_SUPPRESS_MS + tight REFOLLOW during that window), not by a
 * permanent wide hysteresis gap — a permanent 12px re-follow band left the
 * "Jump to latest" chip visible while the user was already visually at bottom.
 */

/** Leave follow / re-enter follow (outside suppress) at this distance. */
export const FOLLOW_THRESHOLD_PX = 80;

/**
 * During the post-wheel-up suppress window only: re-enter follow only when
 * this close to the true bottom, so rubber-band settle mid-gesture does not
 * re-arm follow and fight the next upward frame.
 */
export const REFOLLOW_THRESHOLD_PX = 12;

/** Sub-pixel / layout slack: treat as not overflow unless taller than this. */
export const SCROLLABLE_EPSILON_PX = 1;

/**
 * After a deliberate wheel/trackpad up, suppress mid-band re-follow for this
 * long so pin/ResizeObserver cannot yank the viewport mid-gesture.
 */
export const UNFOLLOW_SUPPRESS_MS = 320;

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

/**
 * True when the viewport can actually move vertically.
 * Short lists (content shorter than the container) are not scrollable even if
 * wheel/trackpad gestures still fire.
 */
export function isVerticallyScrollable(
  el: { scrollHeight: number; clientHeight: number },
  epsilon: number = SCROLLABLE_EPSILON_PX,
): boolean {
  return el.scrollHeight > el.clientHeight + epsilon;
}

/**
 * After a user scroll event, should we be following?
 *
 * Defaults use a single band (FOLLOW_THRESHOLD) both ways so docking at the
 * bottom always re-arms follow. Callers pass a tighter `refollow` only while
 * post-wheel-up suppress is active.
 */
export function followAfterUserScroll(
  distance: number,
  currentlyFollowing: boolean = true,
  thresholds: {
    unfollow?: number;
    refollow?: number;
  } = {},
): boolean {
  const unfollow = thresholds.unfollow ?? FOLLOW_THRESHOLD_PX;
  const refollow = thresholds.refollow ?? FOLLOW_THRESHOLD_PX;
  if (currentlyFollowing) {
    return distance <= unfollow;
  }
  return distance <= refollow;
}

/** Jump chip: hide when following or already within the normal bottom band. */
export function shouldShowJumpToLatest(
  following: boolean,
  distance: number,
  threshold: number = FOLLOW_THRESHOLD_PX,
): boolean {
  if (following) return false;
  return distance > threshold;
}

/**
 * Wheel/trackpad up should break follow only when the list can scroll.
 * Otherwise gestures on a short transcript would show "Jump to latest" with
 * no real scroll offset change.
 */
export function shouldUnfollowOnWheelUp(options: {
  deltaY: number;
  following: boolean;
  scrollable: boolean;
}): boolean {
  return options.following && options.deltaY < 0 && options.scrollable;
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
