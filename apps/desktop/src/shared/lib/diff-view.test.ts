import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  countDiffLines,
  isDiffLike,
  parseDiffLines,
} from "./diff-view.ts";

describe("isDiffLike", () => {
  it("detects unified diff", () => {
    assert.equal(
      isDiffLike("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n"),
      true,
    );
  });

  it("detects apply_patch", () => {
    assert.equal(
      isDiffLike("*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n*** End Patch\n"),
      true,
    );
  });

  it("accepts bare @@ hunk token (parity with TUI/Tauri is_diff_like)", () => {
    assert.equal(isDiffLike("@@\n-old\n+new\n"), true);
    assert.equal(isDiffLike("context\n@@\n-old\n+new\n"), true);
  });

  it("rejects markdown bullets", () => {
    assert.equal(isDiffLike("- just a list item\n- another"), false);
  });

  it("rejects tool args JSON (must not break transcript)", () => {
    assert.equal(
      isDiffLike('{"path":"a.rs","old_string":"-x\\n+y","new_string":"z"}'),
      false,
    );
    assert.equal(
      isDiffLike('{\n  "command": "echo @@ -1 +1 @@"\n}'),
      false,
    );
  });
});

describe("parseDiffLines", () => {
  it("classifies add/del/hunk", () => {
    const lines = parseDiffLines("@@ -1 +1 @@\n-old\n+new\n context\n");
    assert.equal(lines[0]?.kind, "hunk");
    assert.equal(lines[1]?.kind, "del");
    assert.equal(lines[2]?.kind, "add");
    assert.equal(lines[3]?.kind, "context");
  });

  it("counts stats", () => {
    const s = countDiffLines("@@\n-a\n-b\n+c\n");
    assert.deepEqual(s, { add: 1, del: 2 });
  });
});
