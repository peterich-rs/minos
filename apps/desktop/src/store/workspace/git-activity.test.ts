import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  normalizeGitActivity,
  timelineKindForMessage,
} from "./git-activity-map.ts";

describe("normalizeGitActivity", () => {
  it("maps snake_case and camelCase fields", () => {
    const activity = normalizeGitActivity({
      kind: "worktree_created",
      branch: "minos/fix-1",
      worktree_path: "/tmp/.minos-worktrees/fix",
      base_branch: "main",
    });
    assert.deepEqual(activity, {
      kind: "worktree_created",
      branch: "minos/fix-1",
      worktreePath: "/tmp/.minos-worktrees/fix",
      baseBranch: "main",
      count: undefined,
      subjects: undefined,
      head: undefined,
      url: undefined,
      number: undefined,
      title: undefined,
      summary: undefined,
      mergeCommit: undefined,
    });
  });

  it("rejects unknown kinds", () => {
    assert.equal(normalizeGitActivity({ kind: "nope" }), undefined);
    assert.equal(normalizeGitActivity(null), undefined);
  });
});

describe("timelineKindForMessage", () => {
  it("promotes git activity payloads to git_activity kind", () => {
    const activity = normalizeGitActivity({
      kind: "pr_opened",
      url: "https://example.com/pr/3",
      number: 3,
    });
    assert.equal(timelineKindForMessage("text", activity), "git_activity");
    assert.equal(timelineKindForMessage("git_activity", activity), "git_activity");
    assert.equal(timelineKindForMessage("tool_summary", undefined), "tool_summary");
    assert.equal(timelineKindForMessage("text", undefined), "text");
  });
});
