import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  distanceFromBottom,
  followAfterUserScroll,
  followContentKey,
  isNearBottom,
  FOLLOW_THRESHOLD_PX,
} from "./stick-to-bottom.ts";

describe("distanceFromBottom", () => {
  it("returns zero when pinned to bottom", () => {
    assert.equal(
      distanceFromBottom({
        scrollHeight: 1000,
        scrollTop: 700,
        clientHeight: 300,
      }),
      0,
    );
  });

  it("returns positive when scrolled up", () => {
    assert.equal(
      distanceFromBottom({
        scrollHeight: 1000,
        scrollTop: 100,
        clientHeight: 300,
      }),
      600,
    );
  });
});

describe("isNearBottom / followAfterUserScroll", () => {
  it("treats within threshold as near bottom", () => {
    assert.equal(isNearBottom(0), true);
    assert.equal(isNearBottom(FOLLOW_THRESHOLD_PX), true);
    assert.equal(isNearBottom(FOLLOW_THRESHOLD_PX + 1), false);
  });

  it("unfollows when scrolled above threshold", () => {
    assert.equal(followAfterUserScroll(200), false);
    assert.equal(followAfterUserScroll(20), true);
  });
});

describe("followContentKey", () => {
  it("is stable for identical items", () => {
    const items = [{ id: "a", seq: 1, kind: "assistant", text: "hi" }];
    assert.equal(followContentKey(items), followContentKey(items));
  });

  it("changes when last text grows without length change", () => {
    const a = [{ id: "m1", seq: 2, kind: "assistant", text: "hel" }];
    const b = [{ id: "m1", seq: 3, kind: "assistant", text: "hello" }];
    assert.notEqual(followContentKey(a), followContentKey(b));
  });

  it("changes when a new item is appended", () => {
    const a = [{ id: "1", text: "a" }];
    const b = [
      { id: "1", text: "a" },
      { id: "2", text: "b" },
    ];
    assert.notEqual(followContentKey(a), followContentKey(b));
  });

  it("empty list is a fixed key", () => {
    assert.equal(followContentKey([]), "0");
  });
});
