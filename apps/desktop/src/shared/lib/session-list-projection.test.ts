import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  mergeSessionEntity,
  type SessionEntity,
} from "./session-entity.ts";
import {
  mergeRowsIntoProjectSessionList,
  projectEntityIntoLists,
  projectSessionIdsIntoLists,
  rederiveAttentionFromEntities,
  rowsFromEntities,
  type SessionListRow,
} from "./session-list-projection.ts";

function entity(
  id: string,
  status: "running" | "needs_approval" | "failed" | "idle" | "suspended",
  opts?: { pending?: boolean; conversationId?: string },
): SessionEntity {
  return mergeSessionEntity(
    undefined,
    {
      id,
      conversationId: opts?.conversationId ?? "c1",
      agent: "codex",
      shortId: id.slice(0, 4),
      status: status === "needs_approval" ? "running" : status,
      model: "m",
      summary: id,
      lastTsMs: id.charCodeAt(1) * 10,
    },
    {
      pendingApproval: opts?.pending ?? status === "needs_approval",
      nowMs: 1,
    },
  );
}

function row(id: string, status = "running"): SessionListRow {
  return {
    id,
    conversationId: "c1",
    agent: "codex",
    shortId: id.slice(0, 4),
    status,
    model: "m",
    summary: id,
  };
}

describe("projectEntityIntoLists", () => {
  it("updates status in all membership lists that already contain the id", () => {
    const e = entity("s1", "needs_approval");
    const s = {
      sessionsById: { s1: e },
      sessionsByConversation: {
        c1: [row("s1", "running"), row("s2", "idle")],
      },
      projectSessionsByProject: {
        p1: [row("s1", "running")],
      },
      attentionSessions: [row("s1", "running")] as SessionListRow[],
      attentionReady: false,
    };
    const out = projectEntityIntoLists(s, "s1");
    assert.equal(out.sessionsByConversation.c1?.[0]?.status, "needs_approval");
    assert.equal(out.sessionsByConversation.c1?.[1]?.status, "idle");
    assert.equal(
      out.projectSessionsByProject.p1?.[0]?.status,
      "needs_approval",
    );
    assert.equal(out.attentionSessions[0]?.status, "needs_approval");
  });

  it("does not invent membership for lists that lack the session", () => {
    const e = entity("s9", "running");
    const s = {
      sessionsById: { s9: e },
      sessionsByConversation: { c1: [row("s1")] },
      projectSessionsByProject: {},
      attentionSessions: [] as SessionListRow[],
      attentionReady: false,
    };
    const out = projectEntityIntoLists(s, "s9");
    assert.equal(out.sessionsByConversation.c1?.length, 1);
    assert.equal(out.sessionsByConversation.c1?.[0]?.id, "s1");
    assert.deepEqual(out.projectSessionsByProject, {});
    assert.deepEqual(out.attentionSessions, []);
  });

  it("drops attention row when entity no longer needs attention (not ready)", () => {
    const e = entity("s1", "idle", { pending: false });
    const s = {
      sessionsById: { s1: e },
      sessionsByConversation: {},
      projectSessionsByProject: {},
      attentionSessions: [row("s1", "needs_approval")],
      attentionReady: false,
    };
    const out = projectEntityIntoLists(s, "s1");
    assert.equal(out.attentionSessions.length, 0);
  });

  it("does not invent attention row when queue not ready", () => {
    const e = entity("s1", "needs_approval");
    const s = {
      sessionsById: { s1: e },
      sessionsByConversation: {},
      projectSessionsByProject: {},
      attentionSessions: [] as SessionListRow[],
      attentionReady: false,
    };
    const out = projectEntityIntoLists(s, "s1");
    assert.equal(out.attentionSessions.length, 0);
  });

  it("re-derives attention from Entity when attentionReady", () => {
    const s1 = entity("s1", "needs_approval");
    const s2 = entity("s2", "failed");
    const s3 = entity("s3", "idle");
    const s = {
      sessionsById: { s1, s2, s3 },
      sessionsByConversation: {},
      projectSessionsByProject: {},
      // Stale queue missing s2 and still showing s3
      attentionSessions: [row("s1"), row("s3", "failed")],
      attentionReady: true,
    };
    const out = projectEntityIntoLists(s, "s1");
    const ids = out.attentionSessions.map((r) => r.id).sort();
    assert.deepEqual(ids, ["s1", "s2"]);
    assert.equal(
      out.attentionSessions.find((r) => r.id === "s1")?.status,
      "needs_approval",
    );
  });
});

describe("mergeRowsIntoProjectSessionList", () => {
  it("upserts by id and keeps prior project sessions", () => {
    const prev = {
      p1: [
        {
          ...row("old"),
          conversationId: "c-old",
          lastTsMs: 10,
        },
      ],
    };
    const out = mergeRowsIntoProjectSessionList(prev, "p1", [
      {
        ...row("new"),
        conversationId: "c-new",
        lastTsMs: 99,
      },
    ]);
    assert.equal(out.p1!.length, 2);
    assert.ok(out.p1!.some((s) => s.id === "old"));
    assert.ok(out.p1!.some((s) => s.id === "new"));
    assert.equal(out.p1![0]!.id, "new"); // sorted by lastTsMs desc
  });
});

describe("projectSessionIdsIntoLists", () => {
  it("projects sibling lists after bulk hydrate entity upserts", () => {
    const s1 = entity("s1", "needs_approval");
    const s2 = entity("s2", "running");
    const sessionsById = { s1, s2 };
    const s = {
      sessionsById,
      // Inspector already had s1 as running (stale)
      sessionsByConversation: { c1: [row("s1", "running")] },
      projectSessionsByProject: {
        p1: [row("s1", "running"), row("s2", "idle")],
      },
      attentionSessions: [] as SessionListRow[],
      attentionReady: false,
    };
    const out = projectSessionIdsIntoLists(s, ["s1", "s2"]);
    assert.equal(out.sessionsByConversation.c1?.[0]?.status, "needs_approval");
    assert.equal(
      out.projectSessionsByProject.p1?.[0]?.status,
      "needs_approval",
    );
    assert.equal(out.projectSessionsByProject.p1?.[1]?.status, "running");
  });
});

describe("rederiveAttentionFromEntities / rowsFromEntities", () => {
  it("filters and sorts attention-worthy entities", () => {
    const sessionsById = {
      a: entity("a", "idle"),
      b: entity("b", "failed"),
      c: entity("c", "needs_approval"),
    };
    const rows = rederiveAttentionFromEntities(sessionsById);
    assert.deepEqual(
      rows.map((r) => r.id).sort(),
      ["b", "c"],
    );
  });

  it("builds rows from Entity map in order", () => {
    const sessionsById = {
      s1: entity("s1", "running"),
      s2: entity("s2", "needs_approval"),
    };
    const rows = rowsFromEntities(sessionsById, ["s2", "s1"]);
    assert.equal(rows[0]?.id, "s2");
    assert.equal(rows[0]?.status, "needs_approval");
    assert.equal(rows[1]?.status, "running");
  });
});
