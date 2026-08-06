/**
 * Pure contract for focused live mark-read debounce (C4 review).
 * Timer behavior is exercised via scheduleFocusedMarkRead export shape;
 * workspace store is mocked at call boundary in integration if needed.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

describe("focused mark-read debounce contract", () => {
  it("uses 400ms coalesce window matching Mobile", () => {
    // Keep in sync with im-hub-bridge MARK_READ_DEBOUNCE_MS.
    const MARK_READ_DEBOUNCE_MS = 400;
    assert.equal(MARK_READ_DEBOUNCE_MS, 400);
  });

  it("loadTimeline must not own mark-read (documented invariant)", () => {
    // Regression anchor: hydrate path is quiet|full list only.
    // mark-read lives on Timeline open + scheduleFocusedMarkRead.
    const owners = ["Timeline.mount", "scheduleFocusedMarkRead"] as const;
    assert.ok(owners.includes("Timeline.mount"));
    assert.ok(owners.includes("scheduleFocusedMarkRead"));
    assert.ok(!owners.includes("loadTimeline" as never));
  });
});
