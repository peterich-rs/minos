/**
 * Viewport-stable scroll restore for prepend (load-older) without virtualization.
 *
 * Height-delta restore is wrong when stream/layout mutates content while an older
 * page is in flight. Identity-based restore anchors to a stable row id so only
 * that row's movement adjusts scrollTop.
 */

export type ItemScrollAnchor = {
  /** `data-scroll-id` of the row that should stay under the viewport. */
  itemId: string;
  /**
   * Distance from the top of the item to the top of the scrollport
   * (itemRect.top - scrollRect.top). Restoring this keeps the same pixels
   * under the user's eyes after prepend.
   */
  offsetInViewport: number;
};

/**
 * Capture where `itemEl` sits relative to `scrollEl`'s visible top edge.
 * Returns null when the item is missing or not a DOM element.
 */
export function captureItemScrollAnchor(
  scrollEl: HTMLElement,
  itemId: string,
  itemEl: HTMLElement | null | undefined,
): ItemScrollAnchor | null {
  if (!itemEl || !itemId) return null;
  const scrollRect = scrollEl.getBoundingClientRect();
  const itemRect = itemEl.getBoundingClientRect();
  return {
    itemId,
    offsetInViewport: itemRect.top - scrollRect.top,
  };
}

/**
 * After the DOM grows above the anchored item, set scrollTop so the item
 * returns to the same viewport-relative offset.
 * Returns the applied scrollTop, or null if the item is gone.
 */
export function restoreItemScrollAnchor(
  scrollEl: HTMLElement,
  itemEl: HTMLElement | null | undefined,
  anchor: ItemScrollAnchor,
): number | null {
  if (!itemEl) return null;
  const scrollRect = scrollEl.getBoundingClientRect();
  const itemRect = itemEl.getBoundingClientRect();
  const currentOffset = itemRect.top - scrollRect.top;
  const delta = currentOffset - anchor.offsetInViewport;
  if (delta === 0) return scrollEl.scrollTop;
  const next = scrollEl.scrollTop + delta;
  scrollEl.scrollTop = next;
  return next;
}

/** Pure height-delta fallback (tests / environments without item nodes). */
export function scrollTopAfterHeightPrepend(
  prevHeight: number,
  prevTop: number,
  nextHeight: number,
): number {
  const delta = nextHeight - prevHeight;
  return delta > 0 ? prevTop + delta : prevTop;
}

/**
 * Query a scroll-id marker inside the scroll root.
 * Rows should set `data-scroll-id={item.id}` on a stable wrapper.
 */
function escapeAttrSelector(value: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  // Minimal fallback for non-browser unit environments.
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

export function queryScrollItem(
  root: ParentNode | null | undefined,
  itemId: string,
): HTMLElement | null {
  if (!root || !itemId) return null;
  const el = root.querySelector(
    `[data-scroll-id="${escapeAttrSelector(itemId)}"]`,
  );
  return el instanceof HTMLElement ? el : null;
}
