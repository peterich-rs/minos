import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { shouldShowSidebarUpdateCard } from "./sidebarUpdateCardVisibility.ts";

describe("shouldShowSidebarUpdateCard", () => {
  it("shows for actionable or in-flight states", () => {
    for (const state of [
      "ready",
      "installing",
      "manual-required",
      "downloading",
      "available",
    ] as const) {
      assert.equal(shouldShowSidebarUpdateCard({ state }), true, state);
    }
  });

  it("hides for idle/up-to-date/unavailable/error", () => {
    for (const state of [
      "idle",
      "checking",
      "up-to-date",
      "unavailable",
      "error",
    ] as const) {
      assert.equal(shouldShowSidebarUpdateCard({ state }), false, state);
    }
  });
});
