import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  isGrokArrowNumbered,
  parseGrokArrowNumberedLines,
  stripGrokArrowNumbers,
} from "./read-lines.ts";

describe("isGrokArrowNumbered", () => {
  it("detects first-line arrow number", () => {
    assert.equal(isGrokArrowNumbered("880→    foo\n    bar\n890→    baz\n"), true);
  });

  it("rejects plain code", () => {
    assert.equal(isGrokArrowNumbered("const x = 1;\nconst y = 2;\n"), false);
  });

  it("rejects mid-line arrows without leading number", () => {
    assert.equal(isGrokArrowNumbered("a → b\nc → d\n"), false);
  });
});

describe("parseGrokArrowNumberedLines", () => {
  it("expands sparse decade numbering (read_file style)", () => {
    const parsed = parseGrokArrowNumberedLines(
      "880→    }\n  )\n890→  {!following\n",
    );
    assert.ok(parsed);
    assert.deepEqual(parsed, [
      { no: 880, text: "    }" },
      { no: 881, text: "  )" },
      { no: 890, text: "  {!following" },
    ]);
  });

  it("handles dense every-line numbering (write style)", () => {
    const parsed = parseGrokArrowNumberedLines("1→a\n2→b\n3→c\n");
    assert.deepEqual(parsed, [
      { no: 1, text: "a" },
      { no: 2, text: "b" },
      { no: 3, text: "c" },
    ]);
  });
});

describe("stripGrokArrowNumbers", () => {
  it("removes prefixes for clean body text", () => {
    assert.equal(
      stripGrokArrowNumbers("1→hello\nworld\n10→end\n"),
      "hello\nworld\nend",
    );
  });
});
