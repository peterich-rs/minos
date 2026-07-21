/**
 * Stick-to-bottom / follow semantics aligned with TUI ChatState:
 * - following → always pin to max scroll
 * - scroll away from bottom → unfollow
 * - return *very* near bottom → re-follow (hysteresis)
 * - while not following, never programmatically move the viewport
 *
 * Hysteresis (unfollow threshold > re-follow threshold) avoids the macOS
 * rubber-band / pin "rebound" loop: when the user flings to the bottom then
 * immediately scrolls up, a single near-bottom sample must not re-arm follow
 * and fight the next wheel frame.
 */

/** Leave follow when the user is farther from bottom than this. */
export const FOLLOW_THRESHOLD_PX = 80;

/**
 * Re-enter follow only when this close to the true bottom.
 * Much tighter than FOLLOW_THRESHOLD so rubber-band settle does not re-latch
 * while the user is still scrolling up.
 */
export const REFOLLOW_THRESHOLD_PX = 12;

/** Sub-pixel / layout slack: treat as not overflow unless taller than this. */
export const SCROLLABLE_EPSILON_PX = 1;

/**
 * After a deliberate wheel/trackpad up, suppress re-follow for this long so
 * pin/ResizeObserver cannot yank the viewport mid-gesture.
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
 * Uses hysteresis relative to the current follow state.
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
  const refollow = thresholds.refollow ?? REFOLLOW_THRESHOLD_PX;
  if (currentlyFollowing) {
    return distance <= unfollow;
  }
  return distance <= refollow;
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
