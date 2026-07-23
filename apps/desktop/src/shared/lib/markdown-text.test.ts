/**
 * Smoke checks for MarkdownText dependencies (library is unit-tested upstream).
 * Component rendering is covered by tsc + manual UI; node:test has no jsdom.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

describe("markdown stack", () => {
  it("resolves react-markdown and remark-gfm", () => {
    assert.ok(require.resolve("react-markdown"));
    assert.ok(require.resolve("remark-gfm"));
  });
});
