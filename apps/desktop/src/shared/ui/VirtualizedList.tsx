import { type Virtualizer, useVirtualizer } from "@tanstack/react-virtual";
import * as React from "react";
import { cn } from "@/shared/lib/utils";

export type ListVirtualizer = Virtualizer<HTMLElement, Element>;

/**
 * Headless virtualized list (@tanstack/react-virtual), Buzz-shaped API.
 * Rows must tolerate unmount/remount (no DOM-only state).
 */
type VirtualizedListProps<T> = {
  items: T[];
  getItemKey: (item: T, index: number) => string | number;
  renderItem: (item: T, index: number) => React.ReactNode;
  estimateSize?: number;
  overscan?: number;
  scrollRef?: React.RefObject<HTMLElement | null>;
  className?: string;
  innerClassName?: string;
  onVirtualizer?: (virtualizer: ListVirtualizer) => void;
};

export function VirtualizedList<T>({
  items,
  getItemKey,
  renderItem,
  estimateSize = 80,
  overscan = 5,
  scrollRef,
  className,
  innerClassName,
  onVirtualizer,
}: VirtualizedListProps<T>) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const ownsScroll = scrollRef === undefined;
  const resolvedScrollRef = scrollRef ?? internalScrollRef;
  const getScrollElement = React.useCallback(
    () => resolvedScrollRef.current,
    [resolvedScrollRef],
  );

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement,
    estimateSize: () => estimateSize,
    getItemKey: (index) => getItemKey(items[index]!, index),
    overscan,
  });

  React.useLayoutEffect(() => {
    onVirtualizer?.(virtualizer);
  }, [onVirtualizer, virtualizer]);

  const virtualItems = virtualizer.getVirtualItems();

  const content = (
    <div
      className={cn("relative w-full", innerClassName)}
      style={{ height: `${virtualizer.getTotalSize()}px` }}
    >
      {virtualItems.map((virtualRow) => (
        <div
          data-index={virtualRow.index}
          key={virtualRow.key}
          ref={virtualizer.measureElement}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            width: "100%",
            transform: `translateY(${virtualRow.start}px)`,
          }}
        >
          {renderItem(items[virtualRow.index]!, virtualRow.index)}
        </div>
      ))}
    </div>
  );

  if (ownsScroll) {
    return (
      <div
        className={cn("scrollbar-thin min-h-0 overflow-y-auto", className)}
        ref={internalScrollRef}
      >
        {content}
      </div>
    );
  }

  return content;
}
