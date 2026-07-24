import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  applyManagerLifecycleToEntity,
  asDaemonStatus,
  entityNeedsAttention,
  entityUiStatus,
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
    // Daemon still says running; no transcript scan — keep pending.
    const next = mergeSessionEntity(prev, { ...seed, status: "running" }, {
      nowMs: 2,
    });
    assert.equal(next.hasPendingApproval, true);
    assert.equal(next.status, "needs_approval");
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
    const e = patchSessionEntity(undefined, "s9", {
      hasPendingApproval: true,
      daemonStatus: "running",
      conversationId: "c",
    }, { nowMs: 1 });
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
      entityNeedsAttention({ ...e, hasPendingApproval: false, status: "running", parentId: undefined }),
      false,
    );
  });
});
