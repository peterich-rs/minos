import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  matchesProgressFilter,
  progressFilterLabel,
  type ConversationProgressFilter,
} from "./conversation-meta.ts";

describe("matchesProgressFilter", () => {
  it("all matches every progress including missing", () => {
    assert.equal(matchesProgressFilter(undefined, "all"), true);
    assert.equal(matchesProgressFilter("todo", "all"), true);
    assert.equal(matchesProgressFilter("in_progress", "all"), true);
    assert.equal(matchesProgressFilter("done", "all"), true);
  });

  it("treats missing progress as todo", () => {
    assert.equal(matchesProgressFilter(undefined, "todo"), true);
    assert.equal(matchesProgressFilter(undefined, "in_progress"), false);
  });

  it("matches only the selected progress", () => {
    const cases: Array<[ConversationProgressFilter, boolean, boolean]> = [
      ["todo", true, false],
      ["in_progress", false, true],
      ["done", false, false],
    ];
    for (const [filter, expectTodo, expectInProgress] of cases) {
      assert.equal(
        matchesProgressFilter("todo", filter),
        expectTodo,
        `todo vs ${filter}`,
      );
      assert.equal(
        matchesProgressFilter("in_progress", filter),
        expectInProgress,
        `in_progress vs ${filter}`,
      );
    }
  });

  it("folds legacy in_review into In progress", () => {
    assert.equal(matchesProgressFilter("in_review", "in_progress"), true);
    assert.equal(matchesProgressFilter("in_review", "todo"), false);
    assert.equal(matchesProgressFilter("in_review", "done"), false);
    assert.equal(matchesProgressFilter("in_review", "all"), true);
  });
});

describe("progressFilterLabel", () => {
  it("returns human labels for user-facing filters only", () => {
    assert.equal(progressFilterLabel("all"), "All");
    assert.equal(progressFilterLabel("todo"), "To do");
    assert.equal(progressFilterLabel("in_progress"), "In progress");
    assert.equal(progressFilterLabel("done"), "Done");
  });
});
