import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  bumpAccountScopeGeneration,
  getAccountScopeGeneration,
} from "./account-scope-generation.ts";

describe("account scope generation", () => {
  it("bump advances generation for stale async detection", () => {
    const before = getAccountScopeGeneration();
    const next = bumpAccountScopeGeneration();
    assert.equal(next, before + 1);
    assert.equal(getAccountScopeGeneration(), next);
    assert.notEqual(getAccountScopeGeneration(), before);
  });

  it("captured gen becomes stale after leave bump", () => {
    const captured = getAccountScopeGeneration();
    bumpAccountScopeGeneration();
    assert.notEqual(captured, getAccountScopeGeneration());
  });
});
