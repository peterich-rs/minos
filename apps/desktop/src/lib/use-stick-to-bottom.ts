import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  FOLLOW_THRESHOLD_PX,
  distanceFromBottom,
  followAfterUserScroll,
  isVerticallyScrollable,
  shouldUnfollowOnWheelUp,
} from "./stick-to-bottom";

type Options = {
  /** Changes when streamed / appended content should re-pin while following. */
  contentKey: string;
  /** Reset follow when this changes (session / conversation id). */
  resetKey?: string;
  threshold?: number;
};

/**
 * Scroll container stick-to-bottom hook (TUI auto_scroll parity).
 *
 * Attach `scrollRef` to the overflow container and `contentRef` to the
 * growing content root (for ResizeObserver on expand/stream height).
 *
 * Scroll-up must win over programmatic pin: wheel/trackpad up immediately
 * unfollows so long transcripts stay readable while content is still growing.
 */
export function useStickToBottom({
  contentKey,
  resetKey,
  threshold = FOLLOW_THRESHOLD_PX,
}: Options) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  const programmaticRef = useRef(false);
  const programmaticClearTimer = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const setFollowingBoth = useCallback((next: boolean) => {
    if (followingRef.current === next) return;
    followingRef.current = next;
    setFollowing(next);
  }, []);

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

  const pinBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el || !followingRef.current) return;
    markProgrammatic(80);
    el.scrollTop = el.scrollHeight;
  }, [markProgrammatic]);

  const jumpToLatest = useCallback(() => {
    setFollowingBoth(true);
    // pin after follow latches; mark programmatic so the jump does not bounce.
    markProgrammatic(80);
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    // second frame for late layout (markdown / images)
    requestAnimationFrame(() => {
      if (!followingRef.current) return;
      const node = scrollRef.current;
      if (node) {
        markProgrammatic(80);
        node.scrollTop = node.scrollHeight;
      }
    });
  }, [setFollowingBoth, markProgrammatic]);

  // Session / conversation change → re-enter follow and pin before paint
  // so the first frame is already at the tail (avoids top→bottom flash).
  useLayoutEffect(() => {
    setFollowingBoth(true);
    pinBottom();
  }, [resetKey, setFollowingBoth, pinBottom]);

  // User scroll / wheel drives follow on/off.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const onScroll = () => {
      if (programmaticRef.current) return;
      // Short lists never leave the bottom; keep following so the jump chip
      // does not appear when the viewport cannot actually scroll.
      if (!isVerticallyScrollable(el)) {
        setFollowingBoth(true);
        return;
      }
      const next = followAfterUserScroll(distanceFromBottom(el), threshold);
      setFollowingBoth(next);
    };

    // Wheel/trackpad: unfollow *before* scroll settles so pin/ResizeObserver
    // cannot yank the viewport back to bottom mid-gesture — but only when
    // content actually overflows (wheel fires even on non-scrollable lists).
    const onWheel = (e: WheelEvent) => {
      if (
        !shouldUnfollowOnWheelUp({
          deltaY: e.deltaY,
          following: followingRef.current,
          scrollable: isVerticallyScrollable(el),
        })
      ) {
        return;
      }
      programmaticRef.current = false;
      setFollowingBoth(false);
    };

    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("wheel", onWheel, { passive: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("wheel", onWheel);
      if (programmaticClearTimer.current) {
        clearTimeout(programmaticClearTimer.current);
        programmaticClearTimer.current = null;
      }
    };
  }, [threshold, resetKey, setFollowingBoth]);

  // Content identity / body growth while following — layout phase so stream
  // updates do not paint one frame at the previous scroll offset.
  useLayoutEffect(() => {
    if (followingRef.current) {
      pinBottom();
    }
  }, [contentKey, pinBottom]);

  // Height changes (stream growth, expand/collapse) while following.
  useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;

    const ro = new ResizeObserver(() => {
      if (followingRef.current) {
        pinBottom();
      }
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, [resetKey, pinBottom]);

  return {
    scrollRef,
    contentRef,
    following,
    jumpToLatest,
    /**
     * Mark the next scroll events as programmatic (load-older anchor restore).
     * Prevents mid-restore scroll handlers from flipping follow state.
     */
    markProgrammatic,
  };
}
