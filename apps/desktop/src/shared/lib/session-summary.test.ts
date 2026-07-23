import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TranscriptItem } from "./daemon.ts";
import {
  displayPath,
  fileStatsFromPatchBody,
  formatFileChangeLine,
  summarizeSessionFromTranscript,
} from "./session-summary.ts";

function item(
  partial: Partial<TranscriptItem> & Pick<TranscriptItem, "id" | "kind">,
): TranscriptItem {
  return {
    text: "",
    role: null,
    tsMs: 0,
    seq: 0,
    ...partial,
  };
}

describe("fileStatsFromPatchBody", () => {
  it("parses apply_patch multi-file updates", () => {
    const body = `*** Begin Patch
*** Update File: apps/a.ts
@@
-line
+line2
+line3
*** Add File: apps/b.ts
@@
+new
*** End Patch`;
    const stats = fileStatsFromPatchBody(body);
    assert.equal(stats.length, 2);
    assert.equal(stats[0]?.path, "apps/a.ts");
    assert.equal(stats[0]?.del, 1);
    assert.equal(stats[0]?.add, 2);
    assert.equal(stats[1]?.path, "apps/b.ts");
    assert.equal(stats[1]?.add, 1);
  });

  it("parses unified diff --git sections", () => {
    const body = `diff --git a/x.rs b/x.rs
--- a/x.rs
+++ b/x.rs
@@
-old
+new
+new2
diff --git a/y.rs b/y.rs
--- a/y.rs
+++ b/y.rs
@@
-z
`;
    const stats = fileStatsFromPatchBody(body);
    assert.equal(stats.length, 2);
    assert.equal(stats[0]?.path, "x.rs");
    assert.equal(stats[0]?.add, 2);
    assert.equal(stats[0]?.del, 1);
    assert.equal(stats[1]?.path, "y.rs");
    assert.equal(stats[1]?.del, 1);
  });
});

describe("summarizeSessionFromTranscript", () => {
  it("aggregates edit tool results by path", () => {
    const summary = summarizeSessionFromTranscript([
      item({
        id: "1",
        kind: "tool_result",
        title: "search_replace",
        text: "apps/desktop/src/lib/session-list-group.ts",
        detail: "diff +46 -23",
      }),
      item({
        id: "2",
        kind: "tool_result",
        title: "search_replace",
        text: "apps/desktop/src/lib/session-list-group.ts",
        detail: "+2/-1",
      }),
      item({
        id: "3",
        kind: "tool_result",
        title: "read_file",
        text: "README.md",
        detail: "ok",
      }),
    ]);
    assert.equal(summary.files.length, 1);
    assert.equal(summary.files[0]?.path, "apps/desktop/src/lib/session-list-group.ts");
    assert.equal(summary.files[0]?.add, 48);
    assert.equal(summary.files[0]?.del, 24);
    assert.equal(summary.editCallCount, 2);
    assert.equal(summary.toolCallCount, 3);
  });

  it("counts pending edit tools", () => {
    const summary = summarizeSessionFromTranscript([
      item({
        id: "p",
        kind: "tool",
        title: "Write",
        text: "foo.ts",
      }),
    ]);
    assert.equal(summary.pendingEdits, 1);
    assert.equal(summary.files.length, 1);
    assert.equal(summary.files[0]?.path, "foo.ts");
  });

  it("does not count Place+Result twin as two tools or leave pending", () => {
    const summary = summarizeSessionFromTranscript([
      item({
        id: "tool:tc1",
        kind: "tool",
        title: "search_replace",
        text: "a.ts",
        requestId: "tc1",
      }),
      item({
        id: "tool:tc1",
        kind: "tool_result",
        title: "edit: a.ts",
        text: "a.ts",
        requestId: "tc1",
        detail: "diff +2 -1",
      }),
    ]);
    assert.equal(summary.toolCallCount, 1);
    assert.equal(summary.editCallCount, 1);
    assert.equal(summary.pendingEdits, 0);
    assert.equal(summary.files[0]?.add, 2);
    assert.equal(summary.files[0]?.del, 1);
  });

  it("formats display line", () => {
    const line = formatFileChangeLine({
      path: "apps/desktop/src/lib/session-list-group.ts",
      add: 46,
      del: 23,
      ok: true,
      failed: false,
    });
    assert.match(line, /session-list-group\.ts/);
    assert.match(line, /-23/);
    assert.match(line, /\+46/);
  });
});

describe("displayPath", () => {
  it("collapses home and keeps tail segments", () => {
    const p = displayPath(
      "/Users/me/develop/github.com/minos/apps/desktop/src/lib/foo.ts",
      { maxSegments: 3 },
    );
    assert.equal(p, "~/…/src/lib/foo.ts");
  });

  it("keeps short home paths fully under ~", () => {
    assert.equal(
      displayPath("/Users/me/code/Minos/README.md"),
      "~/code/Minos/README.md",
    );
  });
});
