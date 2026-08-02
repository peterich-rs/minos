import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { hubDispatchText } from "./hub-dispatch-text.ts";

describe("hubDispatchText", () => {
  it("prefixes default agent when body has no @mention", () => {
    assert.equal(hubDispatchText("fix the bug", "grok", false), "@grok fix the bug");
  });

  it("is idempotent when @runtime already present", () => {
    assert.equal(hubDispatchText("@grok fix", "grok", false), "@grok fix");
    assert.equal(hubDispatchText("@Grok fix", "grok", false), "@Grok fix");
  });

  it("leaves explicitly routed bodies alone", () => {
    assert.equal(hubDispatchText("fix please", "codex", true), "fix please");
  });

  it("no-ops without agent", () => {
    assert.equal(hubDispatchText("hello", null, false), "hello");
    assert.equal(hubDispatchText("hello", "", false), "hello");
  });
});
