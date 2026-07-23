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
});
