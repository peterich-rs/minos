import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { cn } from "@/shared/lib/utils";

/** Coarse window for plan / large approval bodies (not event-level paging). */
export const DOCUMENT_CHUNK_CHARS = 12_000;

/** Prefetch next window when this many px remain above the bottom. */
const PREFETCH_BOTTOM_PX = 400;

type Props = {
  text: string;
  /** Characters revealed per step. */
  chunkSize?: number;
  className?: string;
  /** Optional wrapper for the visible text (default: monospace pre). */
  renderBody?: (visible: string) => ReactNode;
};

/**
 * Reveal a large string in coarse chunks as the user scrolls (or when the
 * first window does not fill the viewport). Keeps initial mount cheap so a
 * 50–200KB plan does not freeze the modal on open.
 *
 * Data is already in memory (`text`); this is progressive *display*, not a
 * network re-fetch — matching “不必那么细” while still avoiding one-shot
 * full-DOM paint of the whole document.
 */
export function IncrementalText({
  text,
  chunkSize = DOCUMENT_CHUNK_CHARS,
  className,
  renderBody,
}: Props) {
  const body = typeof text === "string" ? text : String(text ?? "");
  const [shown, setShown] = useState(() => Math.min(chunkSize, body.length));
  const scrollerRef = useRef<HTMLDivElement>(null);
  const growingRef = useRef(false);

  // New document → reset window.
  useEffect(() => {
    setShown(Math.min(chunkSize, body.length));
  }, [body, chunkSize]);

  const hasMore = shown < body.length;
  const visible = hasMore ? body.slice(0, shown) : body;

  const grow = useCallback(() => {
    if (!hasMore || growingRef.current) return;
    growingRef.current = true;
    requestAnimationFrame(() => {
      setShown((n) => Math.min(n + chunkSize, body.length));
      growingRef.current = false;
    });
  }, [hasMore, chunkSize, body.length]);

  // Scroll near bottom → next chunk.
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const onScroll = () => {
      const room = el.scrollHeight - el.scrollTop - el.clientHeight;
      if (room < PREFETCH_BOTTOM_PX) grow();
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [grow]);

  // If the first window does not fill the pane, quietly expand a bit so the
  // user is not staring at a half-empty modal (capped steps per paint cycle
  // via grow's rAF + hasMore).
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el || !hasMore) return;
    if (el.scrollHeight <= el.clientHeight + 48) {
      grow();
    }
  }, [shown, hasMore, grow]);

  return (
    <div
      ref={scrollerRef}
      className={cn(
        "scrollbar-thin min-h-0 flex-1 overflow-y-auto overscroll-y-contain",
        className,
      )}
    >
      {renderBody ? (
        renderBody(visible)
      ) : (
        <pre className="whitespace-pre-wrap font-mono text-xs leading-relaxed text-ink-secondary">
          {visible}
        </pre>
      )}
      {hasMore ? (
        <div className="py-2 text-center text-2xs text-ink-muted">
          Scroll for more…
        </div>
      ) : null}
    </div>
  );
}
