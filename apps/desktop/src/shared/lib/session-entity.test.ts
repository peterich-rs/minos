import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  applyManagerLifecycleToEntity,
  asDaemonStatus,
  conversationAggregatesFromEntities,
  daemonStatusFromEntity,
  entityCountsAsApproval,
  entityCountsAsRunning,
  entityNeedsAttention,
  entityUiStatus,
  mergeSampleDaemonStatus,
  mergeSessionEntity,
  patchSessionEntity,
  projectSessionFromEntity,
} from "./session-entity.ts";

const seed = {
  id: "s1",
  conversationId: "c1",
  agent: "grok",
  shortId: "abc",
  status: "running" as const,
  model: "m",
  summary: "hi",
  messageCount: 3,
};

describe("entityUiStatus / asDaemonStatus", () => {
  it("elevates when pending", () => {
    assert.equal(entityUiStatus(true, "running"), "needs_approval");
    assert.equal(entityUiStatus(true, "idle"), "needs_approval");
  });

  it("trusts daemon when not pending", () => {
    assert.equal(entityUiStatus(false, "running"), "running");
    assert.equal(entityUiStatus(false, "done"), "done");
  });

  it("coerces needs_approval wire to running daemon label", () => {
    assert.equal(asDaemonStatus("needs_approval"), "running");
  });
});

describe("daemonStatusFromEntity", () => {
  it("never returns UI needs_approval from Entity.status", () => {
    const e = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    assert.equal(e.status, "needs_approval");
    assert.equal(e.daemonStatus, "running");
    assert.equal(daemonStatusFromEntity(e), "running");
  });
});

describe("mergeSessionEntity", () => {
  it("sets hasPendingApproval and elevates status", () => {
    const e = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    assert.equal(e.hasPendingApproval, true);
    assert.equal(e.status, "needs_approval");
    assert.equal(e.daemonStatus, "running");
  });

  it("elevate-only does not clear pending on false peek", () => {
    const prev = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    const next = mergeSessionEntity(prev, seed, {
      pendingApproval: false,
      approvalPolicy: "elevate-only",
      nowMs: 2,
    });
    assert.equal(next.hasPendingApproval, true);
    assert.equal(next.status, "needs_approval");
  });

  it("sync clears pending and returns daemon status", () => {
    const prev = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    const next = mergeSessionEntity(prev, seed, {
      pendingApproval: false,
      approvalPolicy: "sync",
      nowMs: 2,
    });
    assert.equal(next.hasPendingApproval, false);
    assert.equal(next.status, "running");
  });

  it("preserves pending when signal omitted (list without transcript)", () => {
    const prev = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    const next = mergeSessionEntity(prev, { ...seed, status: "running" }, {
      nowMs: 2,
    });
    assert.equal(next.hasPendingApproval, true);
    assert.equal(next.status, "needs_approval");
  });

  it("does not demote live Entity with stale suspended+needsContinue list seed", () => {
    const prev = mergeSessionEntity(
      undefined,
      { ...seed, status: "running", needsContinue: false },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "suspended",
        needsContinue: true,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "running");
    assert.equal(next.status, "running");
    assert.equal(next.needsContinue, false);
  });

  it("accepts equal-clock idle sample as turn-end against optimistic running", () => {
    // Desktop send stamps lastTsMs slightly ahead; list hydrate may return idle
    // with the same or slightly older last_activity. Still demote so the rail
    // does not stick on Running after the daemon finished.
    const prev = mergeSessionEntity(
      undefined,
      {
        ...seed,
        status: "running",
        needsContinue: false,
        lastTsMs: 1000,
      },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "idle",
        needsContinue: false,
        lastTsMs: 1000,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "idle");
    assert.equal(next.status, "idle");
  });

  it("accepts older-clock idle sample as turn-end against optimistic running", () => {
    const prev = mergeSessionEntity(
      undefined,
      {
        ...seed,
        status: "running",
        needsContinue: false,
        lastTsMs: 2000,
      },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "idle",
        needsContinue: false,
        lastTsMs: 1500,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "idle");
  });

  it("still rejects older suspended sample against live running", () => {
    const prev = mergeSessionEntity(
      undefined,
      {
        ...seed,
        status: "running",
        needsContinue: false,
        lastTsMs: 2000,
      },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "suspended",
        needsContinue: false,
        lastTsMs: 1000,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "running");
  });

  it("accepts idle sample when lastTsMs is newer", () => {
    const prev = mergeSessionEntity(
      undefined,
      {
        ...seed,
        status: "running",
        needsContinue: false,
        lastTsMs: 1000,
      },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "idle",
        needsContinue: false,
        lastTsMs: 2000,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "idle");
    assert.equal(next.status, "idle");
  });

  it("does not resurrect terminal Entity from running sample", () => {
    const prev = mergeSessionEntity(
      undefined,
      { ...seed, status: "done", needsContinue: false, lastTsMs: 50 },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      { ...seed, status: "running", lastTsMs: 40 },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "done");
  });

  it("authoritative source applies sample demote (manager/action)", () => {
    const prev = mergeSessionEntity(
      undefined,
      { ...seed, status: "running", needsContinue: false, lastTsMs: 1000 },
      { nowMs: 1 },
    );
    const next = mergeSessionEntity(
      prev,
      { ...seed, status: "idle", lastTsMs: 1000 },
      { lifecycleSource: "authoritative", nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "idle");
  });

  it("still accepts real suspended demotion without needsContinue", () => {
    const prev = mergeSessionEntity(
      undefined,
      { ...seed, status: "running", needsContinue: false, lastTsMs: 100 },
      { nowMs: 1 },
    );
    // Without newer lastTsMs, older suspended samples stay sticky-live; use
    // a newer clock for a genuine pause.
    const next = mergeSessionEntity(
      prev,
      {
        ...seed,
        status: "suspended",
        needsContinue: false,
        lastTsMs: 200,
      },
      { nowMs: 2 },
    );
    assert.equal(next.daemonStatus, "suspended");
    assert.equal(next.status, "suspended");
  });
});

describe("mergeSampleDaemonStatus", () => {
  it("keeps terminal against older non-terminal", () => {
    const prev = mergeSessionEntity(
      undefined,
      { ...seed, status: "failed", lastTsMs: 10 },
      { nowMs: 1 },
    );
    const m = mergeSampleDaemonStatus(prev, "running", {
      needsContinue: false,
      lastTsMs: 5,
    });
    assert.equal(m.daemonStatus, "failed");
  });
});

describe("applyManagerLifecycleToEntity", () => {
  it("does not demote needs_approval while hasPendingApproval", () => {
    const prev = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    const next = applyManagerLifecycleToEntity(prev, "s1", "running", {
      nowMs: 2,
    });
    assert.equal(next.hasPendingApproval, true);
    assert.equal(next.status, "needs_approval");
    assert.equal(next.daemonStatus, "running");
    assert.equal(next.needsContinue, false);
  });

  it("applies done when pending cleared", () => {
    const prev = mergeSessionEntity(undefined, seed, {
      pendingApproval: false,
      nowMs: 1,
    });
    const next = applyManagerLifecycleToEntity(prev, "s1", "done", {
      nowMs: 2,
    });
    assert.equal(next.status, "done");
  });
});

describe("patchSessionEntity", () => {
  it("creates shell and elevates on hasPendingApproval", () => {
    const e = patchSessionEntity(
      undefined,
      "s9",
      {
        hasPendingApproval: true,
        daemonStatus: "running",
        conversationId: "c",
      },
      { nowMs: 1 },
    );
    assert.equal(e.sessionId, "s9");
    assert.equal(e.status, "needs_approval");
  });
});

describe("projectSessionFromEntity / entityNeedsAttention", () => {
  it("projects status and attention filter", () => {
    const e = mergeSessionEntity(undefined, seed, {
      pendingApproval: true,
      nowMs: 1,
    });
    const row = projectSessionFromEntity(e);
    assert.equal(row.id, "s1");
    assert.equal(row.status, "needs_approval");
    assert.equal(entityNeedsAttention(e), true);
    assert.equal(
      entityNeedsAttention({
        ...e,
        hasPendingApproval: false,
        status: "running",
        parentId: undefined,
      }),
      false,
    );
  });
});

describe("conversationAggregatesFromEntities", () => {
  it("counts running and approval from Entity membership only", () => {
    const a = mergeSessionEntity(
      undefined,
      { ...seed, id: "a", status: "running" },
      { nowMs: 1 },
    );
    const b = mergeSessionEntity(
      undefined,
      { ...seed, id: "b", status: "suspended", needsContinue: true },
      { pendingApproval: false, nowMs: 1 },
    );
    const c = mergeSessionEntity(
      undefined,
      {
        ...seed,
        id: "c",
        conversationId: "other",
        status: "running",
      },
      { nowMs: 1 },
    );
    const pending = mergeSessionEntity(
      undefined,
      { ...seed, id: "d", status: "running" },
      { pendingApproval: true, nowMs: 1 },
    );
    const map = {
      a,
      b,
      c,
      d: pending,
    };
    const agg = conversationAggregatesFromEntities(map, "c1");
    assert.equal(entityCountsAsRunning(a), true);
    assert.equal(entityCountsAsApproval(b), true);
    assert.equal(entityCountsAsRunning(pending), true);
    assert.equal(entityCountsAsApproval(pending), true);
    // a running + d needs_approval (counts as running) = 2
    assert.equal(agg.runningCount, 2);
    // b suspended + d needs_approval = 2
    assert.equal(agg.approvalCount, 2);
  });

  it("ignores subagents for aggregates", () => {
    const parent = mergeSessionEntity(
      undefined,
      { ...seed, id: "p", status: "running" },
      { nowMs: 1 },
    );
    const child = mergeSessionEntity(
      undefined,
      {
        ...seed,
        id: "child",
        status: "running",
        parentId: "p",
      },
      { nowMs: 1 },
    );
    const agg = conversationAggregatesFromEntities(
      { p: parent, child },
      "c1",
    );
    assert.equal(agg.runningCount, 1);
    assert.equal(agg.approvalCount, 0);
  });
});
