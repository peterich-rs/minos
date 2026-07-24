import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { MODAL_EXIT_ANIMATION_MS } from "./deferredModalOpen.ts";

describe("deferredModalOpen", () => {
  it("exposes exit duration matching modal motion closed duration", () => {
    assert.equal(MODAL_EXIT_ANIMATION_MS, 150);
  });
});
