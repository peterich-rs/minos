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

  const pinBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    programmaticRef.current = true;
    el.scrollTop = el.scrollHeight;
    // Let the scroll event from this write settle before re-enabling user detection.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        programmaticRef.current = false;
      });
    });
  }, []);

  const jumpToLatest = useCallback(() => {
    followingRef.current = true;
    setFollowing(true);
    pinBottom();
  }, [pinBottom]);

  // Session / conversation change → re-enter follow.
  useEffect(() => {
    followingRef.current = true;
    setFollowing(true);
  }, [resetKey]);

  // User scroll drives follow on/off (never while we are programmatically pinning).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const onScroll = () => {
      if (programmaticRef.current) return;
      const next = followAfterUserScroll(distanceFromBottom(el), threshold);
      if (next !== followingRef.current) {
        followingRef.current = next;
        setFollowing(next);
      }
    };

    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [threshold, resetKey]);

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
