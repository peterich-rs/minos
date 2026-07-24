import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  childrenOf,
  flattenSessionListRows,
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

describe("flattenSessionListRows", () => {
  it("emits only folder rows when collapsed", () => {
    const groups = groupSessionsByConversation([
      sess({
        id: "a1",
        conversationId: "c1",
        conversationTitle: "One",
        lastTsMs: 10,
      }),
      sess({
        id: "b1",
        conversationId: "c2",
        conversationTitle: "Two",
        lastTsMs: 20,
      }),
    ]);
    const rows = flattenSessionListRows(groups, new Set(["c1", "c2"]));
    assert.equal(rows.length, 2);
    assert.equal(rows[0]?.type, "folder");
    assert.equal(rows[1]?.type, "folder");
  });

  it("DFS expands roots and children when open", () => {
    const groups = groupSessionsByConversation([
      sess({
        id: "root",
        conversationId: "c1",
        conversationTitle: "Tree",
        lastTsMs: 30,
      }),
      sess({
        id: "child",
        conversationId: "c1",
        parentId: "root",
        lastTsMs: 20,
      }),
    ]);
    const rows = flattenSessionListRows(groups, new Set());
    assert.deepEqual(
      rows.map((r) =>
        r.type === "folder"
          ? r.type
          : r.type === "session"
            ? `${r.type}:${r.session.id}:${r.depth}`
            : r.type,
      ),
      ["folder", "session:root:0", "session:child:1"],
    );
  });

  it("emits empty-roots when expanded group has no top-level sessions", () => {
    const groups = groupSessionsByConversation([
      sess({
        id: "only-child",
        conversationId: "c1",
        parentId: "missing",
        lastTsMs: 1,
      }),
    ]);
    const rows = flattenSessionListRows(groups, new Set());
    assert.equal(rows.some((r) => r.type === "empty-roots"), true);
  });
});
