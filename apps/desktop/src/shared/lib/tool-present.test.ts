import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildToolHeader,
  toolHeaderVerb,
  toolKindFromName,
  parseDiffstat,
  collapsedThinkingSummary,
} from "./tool-present.ts";

describe("toolKindFromName", () => {
  it("classifies common tools", () => {
    assert.equal(toolKindFromName("read_file"), "read");
    assert.equal(toolKindFromName("search_replace"), "edit");
    assert.equal(toolKindFromName("run_terminal_command"), "execute");
    assert.equal(toolKindFromName("grep"), "search");
    assert.equal(toolKindFromName("list_dir"), "list");
    assert.equal(toolKindFromName("web_search"), "web_search");
  });
});

describe("toolHeaderVerb", () => {
  it("uses present while running and past when idle", () => {
    assert.equal(toolHeaderVerb("read", true), "Reading");
    assert.equal(toolHeaderVerb("read", false), "Read");
    assert.equal(toolHeaderVerb("execute", true), "Running");
    assert.equal(toolHeaderVerb("execute", false), "Ran");
    assert.equal(toolHeaderVerb("skill", true), "Skill");
  });
});

describe("buildToolHeader", () => {
  it("builds running read header", () => {
    const h = buildToolHeader({
      toolName: "read_file",
      target: "src/main.rs",
      kind: "tool",
    });
    assert.equal(h.verb, "Reading");
    assert.equal(h.target, "src/main.rs");
    assert.equal(h.targetFull, "src/main.rs");
    assert.equal(h.toolKind, "read");
    assert.equal(h.running, true);
    assert.equal(h.failed, false);
  });

  it("collapses home paths in the short target and keeps full in tooltip", () => {
    const full =
      "/Users/fannnzhang/code/github.com/Minos/apps/desktop/src/shared/ui/MarkdownText.tsx";
    const h = buildToolHeader({
      toolName: "read_file",
      target: full,
      kind: "tool_result",
    });
    assert.equal(h.targetFull, full);
    assert.ok(h.target.startsWith("~/"));
    assert.ok(!h.target.includes("/Users/"));
    assert.ok(h.target.endsWith("MarkdownText.tsx"));
    assert.equal(h.toolKind, "read");
  });

  it("builds failed execute header", () => {
    const h = buildToolHeader({
      toolName: "bash",
      target: "cargo test",
      kind: "tool_error",
    });
    assert.equal(h.verb, "Ran");
    assert.equal(h.failed, true);
  });

  it("parses diffstat from detail", () => {
    const h = buildToolHeader({
      toolName: "apply_patch",
      target: "foo.rs",
      kind: "tool_result",
      detail: "diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs\n@@\n+line\n-old\n",
    });
    assert.ok(h.diffstat);
    assert.equal(h.diffstat!.add, 1);
    assert.equal(h.diffstat!.del, 1);
  });

  it("never shows Reading read or XML task titles", () => {
    const readDup = buildToolHeader({
      toolName: "read",
      target: "read",
      kind: "tool",
    });
    assert.equal(readDup.target, "…");

    const xml = buildToolHeader({
      toolName: "task",
      target: '<task id="ses_x" state="completed">',
      kind: "tool_result",
    });
    assert.equal(xml.target, "…");
  });
});

describe("parseDiffstat", () => {
  it("parses +N/-M", () => {
    assert.deepEqual(parseDiffstat("+12/-3"), { add: 12, del: 3 });
  });
});

describe("collapsedThinkingSummary", () => {
  it("truncates long thought", () => {
    const s = collapsedThinkingSummary("a".repeat(120), 40);
    assert.equal(s.length, 41); // 40 + ellipsis
    assert.ok(s.endsWith("…"));
  });
});
