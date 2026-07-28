import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildCreateConversationInput,
  canSubmitCreateConversation,
  defaultCreateConversationForm,
  MAX_AGENT_BRIEF_CHARS,
  normalizeAgentBrief,
  normalizeCreateConversationTitle,
  normalizeGitMode,
  sanitizeSelectedAgents,
  toggleSelectedAgent,
} from "./create-conversation-form.ts";

describe("create-conversation-form", () => {
  it("defaults to empty title, no priority, no agents, worktree git mode", () => {
    assert.deepEqual(defaultCreateConversationForm(), {
      title: "",
      priority: null,
      selectedAgents: [],
      agentBriefs: {},
      gitMode: "worktree",
    });
  });

  it("normalizes title whitespace and requires agents to submit", () => {
    assert.equal(
      normalizeCreateConversationTitle("  Auth   refactor  "),
      "Auth refactor",
    );
    assert.equal(canSubmitCreateConversation("   ", ["codex"]), false);
    assert.equal(canSubmitCreateConversation("  go  ", []), false);
    assert.equal(canSubmitCreateConversation("  go  ", ["codex"]), true);
  });

  it("toggles agents without duplicates", () => {
    assert.deepEqual(toggleSelectedAgent([], "codex"), ["codex"]);
    assert.deepEqual(toggleSelectedAgent(["codex"], "claude"), [
      "codex",
      "claude",
    ]);
    assert.deepEqual(toggleSelectedAgent(["codex", "claude"], "codex"), [
      "claude",
    ]);
    assert.deepEqual(toggleSelectedAgent(["codex"], "  "), ["codex"]);
  });

  it("sanitizes against known options", () => {
    const options = [
      { id: "codex", displayName: "Codex", installed: true },
      { id: "claude", displayName: "Claude", installed: false },
    ];
    assert.deepEqual(
      sanitizeSelectedAgents(["codex", "missing", "codex", "claude"], options),
      ["codex", "claude"],
    );
  });

  it("normalizes git mode", () => {
    assert.equal(normalizeGitMode("worktree"), "worktree");
    assert.equal(normalizeGitMode("inherit"), "inherit");
    assert.equal(normalizeGitMode(null), "worktree");
    assert.equal(normalizeGitMode("nope"), "worktree");
  });

  it("caps agent briefs", () => {
    assert.equal(normalizeAgentBrief("  review  "), "review");
    assert.equal(normalizeAgentBrief("x".repeat(MAX_AGENT_BRIEF_CHARS + 10)).length, MAX_AGENT_BRIEF_CHARS);
  });

  it("builds submit payload with agent briefs or null when title empty", () => {
    assert.equal(
      buildCreateConversationInput({
        title: "  ",
        priority: "high",
        selectedAgents: ["codex"],
        gitMode: "worktree",
      }),
      null,
    );
    assert.deepEqual(
      buildCreateConversationInput({
        title: "  Ship board  ",
        priority: "medium",
        selectedAgents: ["codex", "codex", " claude "],
        agentBriefs: {
          codex: "implements features",
          claude: "  reviews PRs  ",
        },
        gitMode: "inherit",
      }),
      {
        title: "Ship board",
        priority: "medium",
        agents: [
          { agent: "codex", brief: "implements features" },
          { agent: "claude", brief: "reviews PRs" },
        ],
        gitMode: "inherit",
      },
    );
    assert.deepEqual(
      buildCreateConversationInput({
        title: "Solo",
        priority: null,
        selectedAgents: ["codex"],
        agentBriefs: { codex: "   " },
      }),
      {
        title: "Solo",
        priority: null,
        agents: [{ agent: "codex" }],
        gitMode: "worktree",
      },
    );
  });
});
