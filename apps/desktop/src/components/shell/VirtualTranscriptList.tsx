import {
  useEffect,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "@/lib/utils";
import type { TranscriptItem } from "@/lib/daemon";

type Props = {
  items: TranscriptItem[];
  scrollRef: RefObject<HTMLDivElement | null>;
  contentRef: RefObject<HTMLDivElement | null>;
  following: boolean;
  /** Extra nodes before the virtualized list (loading spinner, etc.). */
  header?: ReactNode;
  renderItem: (item: TranscriptItem, index: number) => ReactNode;
  className?: string;
  /** Estimated row height; dynamic measure corrects over time. */
  estimateSize?: number;
  /** Overscan count for smoother scroll while streaming. */
  overscan?: number;
};

/**
 * Virtualized session transcript list.
 *
 * Integrates with stick-to-bottom: the parent owns `scrollRef` / follow state;
 * we measure dynamically and re-pin when `following` + items change.
 */
export function VirtualTranscriptList({
  items,
  scrollRef,
  contentRef,
  following,
  header,
  renderItem,
  className,
  estimateSize = 96,
  overscan = 8,
}: Props) {
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => estimateSize,
    overscan,
    getItemKey: (index) => items[index]?.id ?? index,
  });

  const virtualItems = virtualizer.getVirtualItems();
  const totalSize = virtualizer.getTotalSize();
  const prevCountRef = useRef(items.length);

  // While following: keep the last item in view when the list grows or streams.
  useEffect(() => {
    if (!following || items.length === 0) {
      prevCountRef.current = items.length;
      return;
    }
    const grew = items.length !== prevCountRef.current;
    prevCountRef.current = items.length;
    // Always pin to end while following (stream tail updates + new rows).
    requestAnimationFrame(() => {
      virtualizer.scrollToIndex(items.length - 1, {
        align: "end",
        behavior: grew ? "auto" : "auto",
      });
    });
  }, [following, items.length, items[items.length - 1]?.id, items[items.length - 1]?.text, virtualizer]);

  return (
    <div
      ref={contentRef}
      className={cn("mx-auto w-full max-w-3xl pb-8", className)}
    >
      {header}
      <div
        className="relative w-full"
        style={{ height: `${Math.max(totalSize, 0)}px` }}
      >
        {virtualItems.map((vItem) => {
          const item = items[vItem.index];
          if (!item) return null;
          return (
            <div
              key={vItem.key}
              data-index={vItem.index}
              ref={virtualizer.measureElement}
              className="absolute left-0 top-0 w-full pb-2.5"
              style={{
                transform: `translateY(${vItem.start}px)`,
              }}
            >
              {renderItem(item, vItem.index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
