import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  distanceFromBottom,
  followAfterUserScroll,
  followContentKey,
  isNearBottom,
  isVerticallyScrollable,
  shouldShowJumpToLatest,
  shouldUnfollowOnWheelUp,
  FOLLOW_THRESHOLD_PX,
  REFOLLOW_THRESHOLD_PX,
  SCROLLABLE_EPSILON_PX,
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

  it("unfollows when currently following and scrolled above unfollow threshold", () => {
    assert.equal(followAfterUserScroll(200, true), false);
    assert.equal(followAfterUserScroll(20, true), true);
  });

  it("re-follows in the normal bottom band by default (no permanent gap)", () => {
    // Default: same band both ways so docking at bottom clears Jump.
    const mid = REFOLLOW_THRESHOLD_PX + 20;
    assert.ok(mid < FOLLOW_THRESHOLD_PX);
    assert.equal(followAfterUserScroll(mid, false), true);
    assert.equal(followAfterUserScroll(FOLLOW_THRESHOLD_PX, false), true);
    assert.equal(followAfterUserScroll(FOLLOW_THRESHOLD_PX + 1, false), false);
    assert.equal(followAfterUserScroll(0, false), true);
  });

  it("honors a tight refollow only when caller passes it (suppress window)", () => {
    const mid = REFOLLOW_THRESHOLD_PX + 20;
    assert.equal(
      followAfterUserScroll(mid, false, {
        unfollow: FOLLOW_THRESHOLD_PX,
        refollow: REFOLLOW_THRESHOLD_PX,
      }),
      false,
    );
    assert.equal(
      followAfterUserScroll(REFOLLOW_THRESHOLD_PX, false, {
        unfollow: FOLLOW_THRESHOLD_PX,
        refollow: REFOLLOW_THRESHOLD_PX,
      }),
      true,
    );
  });

  it("stays following until past unfollow threshold", () => {
    assert.equal(followAfterUserScroll(FOLLOW_THRESHOLD_PX, true), true);
    assert.equal(followAfterUserScroll(FOLLOW_THRESHOLD_PX + 1, true), false);
  });
});

describe("shouldShowJumpToLatest", () => {
  it("hides when following", () => {
    assert.equal(shouldShowJumpToLatest(true, 500), false);
  });

  it("hides when already in the bottom band even if unfollowed", () => {
    assert.equal(shouldShowJumpToLatest(false, 0), false);
    assert.equal(shouldShowJumpToLatest(false, FOLLOW_THRESHOLD_PX), false);
  });

  it("shows only when unfollowed and scrolled above the band", () => {
    assert.equal(
      shouldShowJumpToLatest(false, FOLLOW_THRESHOLD_PX + 1),
      true,
    );
  });
});

describe("isVerticallyScrollable", () => {
  it("is false when content fits in the viewport", () => {
    assert.equal(
      isVerticallyScrollable({ scrollHeight: 200, clientHeight: 400 }),
      false,
    );
    assert.equal(
      isVerticallyScrollable({ scrollHeight: 400, clientHeight: 400 }),
      false,
    );
  });

  it("is false for sub-pixel overflow within epsilon", () => {
    assert.equal(
      isVerticallyScrollable({
        scrollHeight: 400 + SCROLLABLE_EPSILON_PX,
        clientHeight: 400,
      }),
      false,
    );
  });

  it("is true when content overflows the viewport", () => {
    assert.equal(
      isVerticallyScrollable({ scrollHeight: 800, clientHeight: 400 }),
      true,
    );
  });
});

describe("shouldUnfollowOnWheelUp", () => {
  it("unfollows only on upward wheel while following a scrollable list", () => {
    assert.equal(
      shouldUnfollowOnWheelUp({
        deltaY: -12,
        following: true,
        scrollable: true,
      }),
      true,
    );
  });

  it("does not unfollow when content cannot scroll (short list)", () => {
    assert.equal(
      shouldUnfollowOnWheelUp({
        deltaY: -40,
        following: true,
        scrollable: false,
      }),
      false,
    );
  });

  it("does not unfollow on wheel down or when already unfollowed", () => {
    assert.equal(
      shouldUnfollowOnWheelUp({
        deltaY: 20,
        following: true,
        scrollable: true,
      }),
      false,
    );
    assert.equal(
      shouldUnfollowOnWheelUp({
        deltaY: -20,
        following: false,
        scrollable: true,
      }),
      false,
    );
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

  it("changes when older items are prepended (length grows, last stays)", () => {
    const tail = [
      { id: "2", seq: 2, kind: "user", text: "mid" },
      { id: "3", seq: 3, kind: "assistant", text: "latest" },
    ];
    const prepended = [
      { id: "1", seq: 1, kind: "user", text: "old" },
      ...tail,
    ];
    // Key must change so layout effects can run, but follow state is independent
    // (hook only pins when followingRef is true — pure content key never forces stick).
    assert.notEqual(followContentKey(tail), followContentKey(prepended));
    assert.equal(followContentKey(tail).split(":")[1], "3");
    assert.equal(followContentKey(prepended).split(":")[1], "3");
  });

  it("empty list is a fixed key", () => {
    assert.equal(followContentKey([]), "0");
  });
});

/**
 * Pure follow-state contract for append/prepend while reading history.
 * Programmatic pin lives in the hook and is gated on followingRef; these
 * helpers only decide whether a scroll sample re-arms follow.
 */
describe("follow state vs content growth (reading history)", () => {
  it("stays unfollowed when reading history even as content key would grow", () => {
    // User scrolled well above the bottom band (reading older messages).
    const readingDistance = FOLLOW_THRESHOLD_PX + 200;
    assert.equal(followAfterUserScroll(readingDistance, false), false);
    // Append / stream growth does not change user-scroll distance math:
    // without a new near-bottom sample, follow must remain off.
    assert.equal(
      followAfterUserScroll(readingDistance, false, {
        unfollow: FOLLOW_THRESHOLD_PX,
        refollow: FOLLOW_THRESHOLD_PX,
      }),
      false,
    );
    assert.equal(
      shouldShowJumpToLatest(false, readingDistance),
      true,
    );
  });

  it("stays following when already at bottom (append may re-pin)", () => {
    assert.equal(followAfterUserScroll(0, true), true);
    assert.equal(followAfterUserScroll(0, false), true);
    assert.equal(shouldShowJumpToLatest(true, 0), false);
  });

  it("prepend while not at bottom does not re-arm follow", () => {
    // Load-older restore keeps viewport on the same row; user is still not
    // near bottom, so follow must stay false (no flip-on from prepend alone).
    const stillReading = FOLLOW_THRESHOLD_PX + 120;
    assert.equal(followAfterUserScroll(stillReading, false), false);
    assert.equal(
      followAfterUserScroll(stillReading, false, {
        unfollow: FOLLOW_THRESHOLD_PX,
        refollow: REFOLLOW_THRESHOLD_PX,
      }),
      false,
    );
  });
});
