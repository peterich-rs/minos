import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { ProjectSession } from "../../../store/workspace/types.ts";
import {
  resolveSessionForView,
  sessionBelongsToProject,
} from "./session-view-resolve.ts";

function session(
  partial: Pick<ProjectSession, "id" | "conversationId"> &
    Partial<ProjectSession>,
): ProjectSession {
  return {
    agent: "opencode",
    shortId: partial.id.slice(0, 8),
    status: "idle",
    model: "m",
    summary: "",
    ...partial,
  };
}

describe("resolveSessionForView", () => {
  const projectId = "proj-a";
  const s1 = session({ id: "sess-1", conversationId: "conv-1" });
  const s2 = session({ id: "sess-2", conversationId: "conv-2" });
  const foreign = session({ id: "sess-x", conversationId: "conv-x" });

  it("returns undefined for null sessionId", () => {
    assert.equal(
      resolveSessionForView(null, projectId, [s1], {}, { "conv-1": projectId }),
      undefined,
    );
  });

  it("returns undefined for empty projectId", () => {
    assert.equal(
      resolveSessionForView("sess-1", "", [s1], {}, { "conv-1": projectId }),
      undefined,
    );
  });

  it("hits from projectSessions first", () => {
    const hit = resolveSessionForView(
      "sess-1",
      projectId,
      [s1, s2],
      {
        "conv-1": [session({ id: "sess-1", conversationId: "conv-1", summary: "other" })],
      },
      { "conv-1": projectId },
    );
    assert.equal(hit, s1);
    assert.equal(hit?.summary, "");
  });

  it("rejects session whose conversation belongs to another project", () => {
    assert.equal(
      resolveSessionForView(
        "sess-x",
        projectId,
        [s1],
        { "conv-x": [foreign] },
        { "conv-x": "proj-other" },
      ),
      undefined,
    );
  });

  it("allows deep-link hit from sessionsByConversation when conv project matches", () => {
    const hit = resolveSessionForView(
      "sess-2",
      projectId,
      [s1],
      { "conv-2": [s2] },
      { "conv-2": projectId },
    );
    assert.equal(hit, s2);
  });

  it("rejects when conv not loaded yet and projectSessions non-empty without matching conversationId", () => {
    assert.equal(
      resolveSessionForView(
        "sess-x",
        projectId,
        [s1],
        { "conv-x": [foreign] },
        {},
      ),
      undefined,
    );
  });

  it("allows deep-link when conv not loaded but projectSessions share conversationId", () => {
    const sibling = session({ id: "sess-sibling", conversationId: "conv-2" });
    const deep = session({ id: "sess-2", conversationId: "conv-2" });
    const hit = resolveSessionForView(
      "sess-2",
      projectId,
      [sibling],
      { "conv-2": [deep] },
      {},
    );
    assert.equal(hit, deep);
  });

  it("allows deep-link when projectSessions empty and conv not loaded yet", () => {
    const hit = resolveSessionForView(
      "sess-2",
      projectId,
      [],
      { "conv-2": [s2] },
      {},
    );
    assert.equal(hit, s2);
  });
});

describe("sessionBelongsToProject", () => {
  const s1 = session({ id: "sess-1", conversationId: "conv-1" });

  it("is false for null sessionId", () => {
    assert.equal(sessionBelongsToProject(null, [s1]), false);
  });

  it("is true when session is in projectSessions", () => {
    assert.equal(sessionBelongsToProject("sess-1", [s1]), true);
  });

  it("is false when session is not in projectSessions", () => {
    assert.equal(sessionBelongsToProject("sess-missing", [s1]), false);
  });
});
