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

  it("returns null when item element is missing", () => {
    const scrollEl = {
      getBoundingClientRect: () => ({ top: 0 }),
      scrollTop: 0,
    } as unknown as HTMLElement;
    assert.equal(
      captureItemScrollAnchor(scrollEl, "x", null),
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
