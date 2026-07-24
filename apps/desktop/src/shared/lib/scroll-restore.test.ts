import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  scrollTopAfterHeightPrepend,
  captureItemScrollAnchor,
  restoreItemScrollAnchor,
} from "./scroll-restore.ts";

describe("scrollTopAfterHeightPrepend", () => {
  it("shifts top by the height growth", () => {
    assert.equal(scrollTopAfterHeightPrepend(1000, 200, 1400), 600);
  });

  it("does not move when height did not grow", () => {
    assert.equal(scrollTopAfterHeightPrepend(1000, 200, 1000), 200);
    assert.equal(scrollTopAfterHeightPrepend(1000, 200, 900), 200);
  });
});

describe("capture / restore item scroll anchor (layout geometry)", () => {
  it("computes viewport-relative offset from getBoundingClientRect", () => {
    const scrollEl = {
      getBoundingClientRect: () => ({ top: 100, left: 0, bottom: 500, right: 0 }),
      scrollTop: 40,
    } as unknown as HTMLElement;
    const itemEl = {
      getBoundingClientRect: () => ({ top: 180, left: 0, bottom: 220, right: 0 }),
    } as unknown as HTMLElement;

    const anchor = captureItemScrollAnchor(scrollEl, "msg-1", itemEl);
    assert.deepEqual(anchor, { itemId: "msg-1", offsetInViewport: 80 });
  });

  it("restores by adjusting scrollTop so offset matches prior", () => {
    let scrollTop = 40;
    const scrollEl = {
      getBoundingClientRect: () => ({ top: 100, left: 0, bottom: 500, right: 0 }),
      get scrollTop() {
        return scrollTop;
      },
      set scrollTop(v: number) {
        scrollTop = v;
      },
    } as unknown as HTMLElement;
    // After prepend the item moved down by 300px in the content.
    const itemEl = {
      getBoundingClientRect: () => ({ top: 480, left: 0, bottom: 520, right: 0 }),
    } as unknown as HTMLElement;

    const next = restoreItemScrollAnchor(scrollEl, itemEl, {
      itemId: "msg-1",
      offsetInViewport: 80,
    });
    // currentOffset = 480 - 100 = 380; delta = 380 - 80 = 300 → scrollTop 340
    assert.equal(next, 340);
    assert.equal(scrollTop, 340);
  });

  it("capture before prepend then restore keeps the same viewport offset", () => {
    // Simulate: item sits 80px below scrollport top; prepend inserts 250px above.
    const scrollRectTop = 100;
    let itemTop = 180;
    let scrollTop = 120;
    const scrollEl = {
      getBoundingClientRect: () => ({
        top: scrollRectTop,
        left: 0,
        bottom: 500,
        right: 0,
      }),
      get scrollTop() {
        return scrollTop;
      },
      set scrollTop(v: number) {
        scrollTop = v;
        // Applying scrollTop moves content up; item's client rect shifts by -delta.
        // Test harness applies the geometric effect of the write.
      },
    } as unknown as HTMLElement;
    const itemEl = {
      getBoundingClientRect: () => ({
        top: itemTop,
        left: 0,
        bottom: itemTop + 40,
        right: 0,
      }),
    } as unknown as HTMLElement;

    const anchor = captureItemScrollAnchor(scrollEl, "anchor-row", itemEl);
    assert.deepEqual(anchor, {
      itemId: "anchor-row",
      offsetInViewport: 80,
    });

    // DOM grows above the row (older page prepended) before we restore.
    const prependHeight = 250;
    itemTop += prependHeight;
    // Without restore, offset would be 180+250-100 = 330.
    assert.equal(itemTop - scrollRectTop, 330);

    const applied = restoreItemScrollAnchor(scrollEl, itemEl, anchor!);
    assert.equal(applied, 120 + prependHeight);
    assert.equal(scrollTop, 370);

    // After the browser applies scrollTop, item rect moves up by the same delta.
    itemTop -= prependHeight;
    assert.equal(itemTop - scrollRectTop, anchor!.offsetInViewport);
  });

  it("is a no-op when the item already sits at the anchored offset", () => {
    let scrollTop = 55;
    const scrollEl = {
      getBoundingClientRect: () => ({ top: 0, left: 0, bottom: 400, right: 0 }),
      get scrollTop() {
        return scrollTop;
      },
      set scrollTop(v: number) {
        scrollTop = v;
      },
    } as unknown as HTMLElement;
    const itemEl = {
      getBoundingClientRect: () => ({ top: 40, left: 0, bottom: 80, right: 0 }),
    } as unknown as HTMLElement;

    const next = restoreItemScrollAnchor(scrollEl, itemEl, {
      itemId: "stable",
      offsetInViewport: 40,
    });
    assert.equal(next, 55);
    assert.equal(scrollTop, 55);
  });

  it("returns null when item element is missing or itemId is empty", () => {
    const scrollEl = {
      getBoundingClientRect: () => ({ top: 0 }),
      scrollTop: 0,
    } as unknown as HTMLElement;
    assert.equal(
      captureItemScrollAnchor(scrollEl, "x", null),
      null,
    );
    assert.equal(
      captureItemScrollAnchor(
        scrollEl,
        "",
        {
          getBoundingClientRect: () => ({ top: 10 }),
        } as unknown as HTMLElement,
      ),
      null,
    );
    assert.equal(
      restoreItemScrollAnchor(scrollEl, null, {
        itemId: "x",
        offsetInViewport: 0,
      }),
      null,
    );
  });
});
