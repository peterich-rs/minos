import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MutableRefObject,
  type RefObject,
} from "react";
import {
  FOLLOW_THRESHOLD_PX,
  REFOLLOW_THRESHOLD_PX,
  UNFOLLOW_SUPPRESS_MS,
  distanceFromBottom,
  followAfterUserScroll,
  isVerticallyScrollable,
  shouldShowJumpToLatest,
  shouldUnfollowOnWheelUp,
} from "./stick-to-bottom";

type Options = {
  /** Changes when streamed / appended content should re-pin while following. */
  contentKey: string;
  /** Reset follow when this changes (session / conversation id). */
  resetKey?: string;
  threshold?: number;
  /**
   * When true, contentKey / ResizeObserver must not pin (e.g. load-older
   * identity restore owns scrollTop for this layout pass).
   */
  pinSuspendedRef?: RefObject<boolean> | MutableRefObject<boolean>;
};

/**
 * Scroll container stick-to-bottom hook (TUI auto_scroll parity).
 *
 * Attach `scrollRef` to the overflow container and `contentRef` to the
 * growing content root (for ResizeObserver on expand/stream height).
 *
 * Scroll-up must win over programmatic pin: wheel/trackpad up immediately
 * unfollows and suppresses re-follow/pin for a short window so rubber-band
 * settle + stream growth cannot yank the viewport mid-gesture.
 *
 * Pin is coalesced to one rAF per frame so contentKey + ResizeObserver do not
 * fight each other (or load-older restore) with multiple scrollTop writes.
 */
export function useStickToBottom({
  contentKey,
  resetKey,
  threshold = FOLLOW_THRESHOLD_PX,
  pinSuspendedRef,
}: Options) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  /** UI for Jump chip — false when following or already in the bottom band. */
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  const followingRef = useRef(true);
  const programmaticRef = useRef(false);
  const programmaticClearTimer = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const pinRafRef = useRef<number | null>(null);
  /** performance.now() until which mid-band re-follow + pin are suppressed after wheel-up. */
  const suppressRefollowUntilRef = useRef(0);
  const suppressSettleTimer = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const setFollowingBoth = useCallback((next: boolean) => {
    if (followingRef.current === next) return;
    followingRef.current = next;
    setFollowing(next);
    if (next) setShowJumpToLatest(false);
  }, []);

  const syncJumpVisibility = useCallback(
    (dist: number, isFollowing: boolean) => {
      setShowJumpToLatest(shouldShowJumpToLatest(isFollowing, dist, threshold));
    },
    [threshold],
  );

  const markProgrammatic = useCallback((ms = 80) => {
    programmaticRef.current = true;
    // WKWebView can deliver the synthetic scroll event after double-rAF;
    // hold the guard so pin / anchor restore does not re-arm follow incorrectly.
    if (programmaticClearTimer.current) {
      clearTimeout(programmaticClearTimer.current);
    }
    programmaticClearTimer.current = setTimeout(() => {
      programmaticRef.current = false;
      programmaticClearTimer.current = null;
    }, ms);
  }, []);

  const cancelScheduledPin = useCallback(() => {
    if (pinRafRef.current != null) {
      cancelAnimationFrame(pinRafRef.current);
      pinRafRef.current = null;
    }
  }, []);

  const isPinSuppressed = useCallback(() => {
    if (pinSuspendedRef?.current) return true;
    if (performance.now() < suppressRefollowUntilRef.current) return true;
    return false;
  }, [pinSuspendedRef]);

  const pinBottomNow = useCallback(() => {
    const el = scrollRef.current;
    if (!el || !followingRef.current) return;
    if (isPinSuppressed()) return;
    markProgrammatic(80);
    el.scrollTop = el.scrollHeight;
  }, [markProgrammatic, isPinSuppressed]);

  /** Coalesce pin to a single layout frame. */
  const schedulePinBottom = useCallback(() => {
    if (!followingRef.current) return;
    if (isPinSuppressed()) return;
    if (pinRafRef.current != null) return;
    pinRafRef.current = requestAnimationFrame(() => {
      pinRafRef.current = null;
      pinBottomNow();
    });
  }, [pinBottomNow, isPinSuppressed]);

  const jumpToLatest = useCallback(() => {
    cancelScheduledPin();
    suppressRefollowUntilRef.current = 0;
    setFollowingBoth(true);
    // pin after follow latches; mark programmatic so the jump does not bounce.
    markProgrammatic(80);
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    // second frame for late layout (markdown / images) — only if still following
    // and user has not started scrolling up.
    requestAnimationFrame(() => {
      if (!followingRef.current) return;
      if (isPinSuppressed()) return;
      const node = scrollRef.current;
      if (node) {
        markProgrammatic(80);
        node.scrollTop = node.scrollHeight;
      }
    });
  }, [
    setFollowingBoth,
    markProgrammatic,
    cancelScheduledPin,
    isPinSuppressed,
  ]);

  // Session / conversation change → re-enter follow and pin before paint
  // so the first frame is already at the tail (avoids top→bottom flash).
  useLayoutEffect(() => {
    cancelScheduledPin();
    suppressRefollowUntilRef.current = 0;
    setFollowingBoth(true);
    pinBottomNow();
  }, [resetKey, setFollowingBoth, pinBottomNow, cancelScheduledPin]);

  // User scroll / wheel drives follow on/off.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const applyFollowFromDistance = (dist: number) => {
      const now = performance.now();
      const suppressed = now < suppressRefollowUntilRef.current;
      // During suppress: only re-latch at the true bottom (tight band).
      // Outside suppress: same band both ways so docking at bottom always
      // clears the Jump chip.
      const next = followAfterUserScroll(dist, followingRef.current, {
        unfollow: threshold,
        refollow: suppressed ? REFOLLOW_THRESHOLD_PX : threshold,
      });
      setFollowingBoth(next);
      syncJumpVisibility(dist, next);
    };

    const scheduleSuppressSettle = () => {
      if (suppressSettleTimer.current) {
        clearTimeout(suppressSettleTimer.current);
      }
      const delay = Math.max(
        0,
        suppressRefollowUntilRef.current - performance.now(),
      );
      suppressSettleTimer.current = setTimeout(() => {
        suppressSettleTimer.current = null;
        const node = scrollRef.current;
        if (!node || programmaticRef.current) return;
        if (!isVerticallyScrollable(node)) {
          setFollowingBoth(true);
          return;
        }
        // Last scroll sample may have landed during suppress; re-sample once
        // the window ends so a settled bottom does not leave Jump stuck on.
        applyFollowFromDistance(distanceFromBottom(node));
      }, delay + 16);
    };

    const onScroll = () => {
      if (programmaticRef.current) return;
      // Short lists never leave the bottom; keep following so the jump chip
      // does not appear when the viewport cannot actually scroll.
      if (!isVerticallyScrollable(el)) {
        setFollowingBoth(true);
        setShowJumpToLatest(false);
        return;
      }
      applyFollowFromDistance(distanceFromBottom(el));
    };

    // Wheel/trackpad: unfollow *before* scroll settles so pin/ResizeObserver
    // cannot yank the viewport back to bottom mid-gesture — but only when
    // content actually overflows (wheel fires even on non-scrollable lists).
    const onWheel = (e: WheelEvent) => {
      // Toward bottom: drop suppress so intentional fling-to-tail re-follows.
      if (e.deltaY > 0) {
        suppressRefollowUntilRef.current = 0;
        if (suppressSettleTimer.current) {
          clearTimeout(suppressSettleTimer.current);
          suppressSettleTimer.current = null;
        }
        return;
      }

      if (
        !shouldUnfollowOnWheelUp({
          deltaY: e.deltaY,
          following: followingRef.current,
          scrollable: isVerticallyScrollable(el),
        })
      ) {
        // Already unfollowed but still scrolling up — keep mid-band re-follow
        // blocked briefly (rubber-band from near bottom).
        if (e.deltaY < 0 && isVerticallyScrollable(el)) {
          suppressRefollowUntilRef.current =
            performance.now() + UNFOLLOW_SUPPRESS_MS;
          cancelScheduledPin();
          programmaticRef.current = false;
          scheduleSuppressSettle();
        }
        return;
      }
      programmaticRef.current = false;
      suppressRefollowUntilRef.current =
        performance.now() + UNFOLLOW_SUPPRESS_MS;
      cancelScheduledPin();
      setFollowingBoth(false);
      syncJumpVisibility(distanceFromBottom(el), false);
      scheduleSuppressSettle();
    };

    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("wheel", onWheel, { passive: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("wheel", onWheel);
      cancelScheduledPin();
      if (suppressSettleTimer.current) {
        clearTimeout(suppressSettleTimer.current);
        suppressSettleTimer.current = null;
      }
      if (programmaticClearTimer.current) {
        clearTimeout(programmaticClearTimer.current);
        programmaticClearTimer.current = null;
      }
    };
  }, [
    threshold,
    resetKey,
    setFollowingBoth,
    cancelScheduledPin,
    syncJumpVisibility,
  ]);

  // Content identity / body growth while following — layout phase so stream
  // updates do not paint one frame at the previous scroll offset.
  useLayoutEffect(() => {
    if (followingRef.current) {
      schedulePinBottom();
    }
  }, [contentKey, schedulePinBottom]);

  // Height changes (stream growth, expand/collapse) while following.
  useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;

    const ro = new ResizeObserver(() => {
      if (followingRef.current) {
        schedulePinBottom();
      }
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, [resetKey, schedulePinBottom]);

  return {
    scrollRef,
    contentRef,
    following,
    /** Show “Jump to latest” only when unfollowed and not near the bottom. */
    showJumpToLatest,
    /** Live follow flag for async load-older (avoids stale React state). */
    followingRef,
    jumpToLatest,
    /**
     * Mark the next scroll events as programmatic (load-older anchor restore).
     * Prevents mid-restore scroll handlers from flipping follow state.
     */
    markProgrammatic,
    /** Immediate pin (skips rAF); still respects follow + pinSuspended. */
    pinBottomNow,
    cancelScheduledPin,
  };
}
