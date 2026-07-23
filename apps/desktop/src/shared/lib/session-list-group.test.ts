import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  childrenOf,
  groupSessionsByConversation,
  sessionIsExecuting,
} from "./session-list-group.ts";
import type { ProjectSession } from "../../store/workspace-store.ts";

function sess(
  partial: Partial<ProjectSession> &
    Pick<ProjectSession, "id" | "conversationId">,
): ProjectSession {
  return {
    agent: "codex",
    shortId: partial.id.slice(0, 4),
    status: "idle",
    model: "—",
    summary: "summary",
    ...partial,
  };
}

describe("groupSessionsByConversation", () => {
  it("groups by conversation and sorts by last activity", () => {
    const groups = groupSessionsByConversation([
      sess({
        id: "a1",
        conversationId: "c-old",
        conversationTitle: "Old",
        lastTsMs: 100,
      }),
      sess({
        id: "b1",
        conversationId: "c-new",
        conversationTitle: "New",
        lastTsMs: 500,
        status: "running",
      }),
      sess({
        id: "b2",
        conversationId: "c-new",
        conversationTitle: "New",
        lastTsMs: 400,
        parentId: "b1",
      }),
    ]);
    assert.equal(groups.length, 2);
    assert.equal(groups[0]?.conversationId, "c-new");
    assert.equal(groups[0]?.roots.length, 1);
    assert.equal(groups[0]?.roots[0]?.id, "b1");
    assert.equal(groups[0]?.runningCount, 1);
    assert.equal(groups[1]?.conversationId, "c-old");
  });

  it("treats missing title as Untitled conversation", () => {
    const groups = groupSessionsByConversation([
      sess({ id: "x", conversationId: "c1", lastTsMs: 1 }),
    ]);
    assert.equal(groups[0]?.title, "Untitled conversation");
  });
});

describe("childrenOf", () => {
  it("returns direct children sorted by activity", () => {
    const all = [
      sess({ id: "p", conversationId: "c", lastTsMs: 10 }),
      sess({
        id: "c2",
        conversationId: "c",
        parentId: "p",
        lastTsMs: 20,
      }),
      sess({
        id: "c1",
        conversationId: "c",
        parentId: "p",
        lastTsMs: 30,
      }),
    ];
    const kids = childrenOf("p", all);
    assert.deepEqual(
      kids.map((k) => k.id),
      ["c1", "c2"],
    );
  });
});

describe("sessionIsExecuting", () => {
  it("flags running and needs_approval", () => {
    assert.equal(sessionIsExecuting("running"), true);
    assert.equal(sessionIsExecuting("needs_approval"), true);
    assert.equal(sessionIsExecuting("idle"), false);
    assert.equal(sessionIsExecuting("done"), false);
  });
});
