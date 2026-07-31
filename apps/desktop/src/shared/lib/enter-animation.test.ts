import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { nextEnterAnimationIds } from "./enter-animation.ts";

describe("nextEnterAnimationIds", () => {
  it("first paint marks all seen and animates none", () => {
    const { nextSeen, animateIds } = nextEnterAnimationIds(new Set(), [
      "a",
      "b",
      "c",
    ]);
    assert.deepEqual([...nextSeen].sort(), ["a", "b", "c"]);
    assert.equal(animateIds.size, 0);
  });

  it("only new ids animate on subsequent updates", () => {
    const prev = new Set(["a", "b"]);
    const { nextSeen, animateIds } = nextEnterAnimationIds(prev, [
      "a",
      "b",
      "c",
    ]);
    assert.deepEqual([...animateIds], ["c"]);
    assert.ok(nextSeen.has("c"));
    assert.ok(nextSeen.has("a"));
  });

  it("prepend older history does not re-animate existing ids", () => {
    const prev = new Set(["b", "c"]);
    // older page adds "a" at front — still new id, animates once is acceptable;
    // we only care that "b"/"c" do not re-enter.
    const { animateIds } = nextEnterAnimationIds(prev, ["a", "b", "c"]);
    assert.ok(animateIds.has("a"));
    assert.ok(!animateIds.has("b"));
    assert.ok(!animateIds.has("c"));
  });

  it("bulk first paint of long history animates none", () => {
    const ids = Array.from({ length: 40 }, (_, i) => `m${i}`);
    const { animateIds, nextSeen } = nextEnterAnimationIds(new Set(), ids);
    assert.equal(animateIds.size, 0);
    assert.equal(nextSeen.size, 40);
  });

  it("streaming multi-append only animates the new tail ids", () => {
    const prev = new Set(["a", "b", "c"]);
    const { animateIds } = nextEnterAnimationIds(prev, [
      "a",
      "b",
      "c",
      "d",
      "e",
    ]);
    assert.deepEqual([...animateIds].sort(), ["d", "e"]);
  });

  it("re-render with identical ids animates none", () => {
    const prev = new Set(["a", "b"]);
    const { animateIds, nextSeen } = nextEnterAnimationIds(prev, ["a", "b"]);
    assert.equal(animateIds.size, 0);
    assert.equal(nextSeen.size, 2);
  });
});
