import { useCallback, useEffect, useRef, useState } from "react";
import {
  FOLLOW_THRESHOLD_PX,
  distanceFromBottom,
  followAfterUserScroll,
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

  const pinBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el || !followingRef.current) return;
    programmaticRef.current = true;
    el.scrollTop = el.scrollHeight;
    // WKWebView can deliver the synthetic scroll event after double-rAF;
    // hold the guard a bit longer so pin does not re-arm follow incorrectly.
    if (programmaticClearTimer.current) {
      clearTimeout(programmaticClearTimer.current);
    }
    programmaticClearTimer.current = setTimeout(() => {
      programmaticRef.current = false;
      programmaticClearTimer.current = null;
    }, 50);
  }, []);

  const jumpToLatest = useCallback(() => {
    setFollowingBoth(true);
    pinBottom();
  }, [setFollowingBoth, pinBottom]);

  // Session / conversation change → re-enter follow.
  useEffect(() => {
    setFollowingBoth(true);
  }, [resetKey, setFollowingBoth]);

  // User scroll / wheel drives follow on/off.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const onScroll = () => {
      if (programmaticRef.current) return;
      const next = followAfterUserScroll(distanceFromBottom(el), threshold);
      setFollowingBoth(next);
    };

    // Wheel/trackpad: unfollow *before* scroll settles so pin/ResizeObserver
    // cannot yank the viewport back to bottom mid-gesture.
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0 && followingRef.current) {
        programmaticRef.current = false;
        setFollowingBoth(false);
      }
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

  // Content identity / body growth while following.
  useEffect(() => {
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
  };
}
