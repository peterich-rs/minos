import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  classifyOutboxFailure,
  enqueueAgentResult,
  enqueueApprovalResolve,
  enqueueReactionToggle,
  enqueueUserMessage,
  forceUpdatedAtForTests,
  getOutboxSnapshotForTests,
  isAcked,
  listDuePending,
  markAcked,
  markFailed,
  markInflight,
  reclaimStaleInflight,
  resetImOutboxForTests,
  STALE_INFLIGHT_MS,
} from "./im-outbox.ts";

// Minimal localStorage for node:test
const mem = new Map<string, string>();
(globalThis as { localStorage?: Storage }).localStorage = {
  getItem: (k: string) => mem.get(k) ?? null,
  setItem: (k: string, v: string) => {
    mem.set(k, v);
  },
  removeItem: (k: string) => {
    mem.delete(k);
  },
  clear: () => mem.clear(),
  key: () => null,
  get length() {
    return mem.size;
  },
} as Storage;

describe("im-outbox", () => {
  beforeEach(() => {
    mem.clear();
    resetImOutboxForTests();
  });

  it("enqueues pending user messages and acks prevent re-project", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m1",
      text: "hello",
    });
    assert.equal(isAcked("m1"), false);
    const due = listDuePending();
    assert.equal(due.length, 1);
    assert.equal(due[0]!.clientMessageId, "m1");

    markInflight("m1");
    markAcked("m1");
    assert.equal(isAcked("m1"), true);
    assert.equal(listDuePending().length, 0);

    // Re-enqueue after ack is a no-op for status
    const again = enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m1",
      text: "hello",
    });
    assert.equal(again.status, "acked");
  });

  it("network errors stay pending after many attempts (no terminal burn)", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m2",
      text: "x",
    });
    for (let i = 0; i < 20; i++) {
      markInflight("m2");
      markFailed("m2", "network error: ECONNRESET");
    }
    const snap = getOutboxSnapshotForTests();
    const row = snap.find((e) => e.clientMessageId === "m2");
    assert.ok(row);
    assert.equal(row!.status, "pending");
    // Due later after backoff, not terminal.
    assert.equal(row!.status, "pending");
  });

  it("permanent client errors terminal after enough attempts", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m-perm",
      text: "x",
    });
    for (let i = 0; i < 8; i++) {
      markInflight("m-perm");
      markFailed("m-perm", "HTTP 400 bad request");
    }
    const row = getOutboxSnapshotForTests().find(
      (e) => e.clientMessageId === "m-perm",
    );
    assert.ok(row);
    assert.equal(row!.status, "failed_terminal");
    assert.equal(listDuePending().length, 0);
  });

  it("classifyOutboxFailure separates transient vs permanent", () => {
    assert.equal(classifyOutboxFailure("network"), "transient");
    assert.equal(classifyOutboxFailure("Not signed in"), "transient");
    assert.equal(classifyOutboxFailure("connection refused"), "transient");
    assert.equal(classifyOutboxFailure("HTTP 503"), "transient");
    assert.equal(classifyOutboxFailure("HTTP 400"), "permanent");
    assert.equal(classifyOutboxFailure("invalid_payload_json"), "permanent");
    assert.equal(classifyOutboxFailure("HTTP 429 rate limit"), "transient");
  });

  it("reclaims stale inflight so kill mid-flight becomes due again", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m-stale",
      text: "in flight",
    });
    markInflight("m-stale");
    const snap = getOutboxSnapshotForTests();
    const row = snap.find((e) => e.clientMessageId === "m-stale");
    assert.equal(row!.status, "inflight");

    // Fresh inflight is not due
    assert.equal(listDuePending().length, 0);

    // Simulate process kill: updatedAt far in the past
    const old = Date.now() - STALE_INFLIGHT_MS - 1_000;
    forceUpdatedAtForTests("m-stale", old);

    const reclaimed = reclaimStaleInflight();
    assert.equal(reclaimed, 1);
    const after = getOutboxSnapshotForTests().find(
      (e) => e.clientMessageId === "m-stale",
    );
    assert.equal(after!.status, "pending");
    assert.equal(listDuePending().length, 1);
    assert.equal(listDuePending()[0]!.clientMessageId, "m-stale");
  });

  it("listDuePending includes reclaim of stale inflight inline", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m-inline",
      text: "x",
    });
    markInflight("m-inline");
    forceUpdatedAtForTests("m-inline", Date.now() - STALE_INFLIGHT_MS - 5_000);
    const due = listDuePending();
    assert.equal(due.length, 1);
    assert.equal(due[0]!.clientMessageId, "m-inline");
    assert.equal(due[0]!.status, "pending");
  });

  it("enqueues agent_result kind into the same status machine", () => {
    const entry = enqueueAgentResult({
      conversationId: "c1",
      clientMessageId: "agent-result:c1:s1:origin1",
      text: "done",
      agentId: "agent-cloud-1",
      agentSessionId: "s1",
    });
    assert.equal(entry.kind, "agent_result");
    assert.equal(entry.status, "pending");
    assert.equal(entry.messageSource, "host_projection");
    assert.equal(listDuePending().length, 1);
    assert.equal(listDuePending()[0]!.kind, "agent_result");

    markInflight(entry.clientMessageId);
    markAcked(entry.clientMessageId);
    assert.equal(isAcked(entry.clientMessageId), true);
    assert.equal(listDuePending().length, 0);
  });

  it("includes reaction_toggle in due queue (C5.1)", () => {
    enqueueReactionToggle({
      conversationId: "c1",
      clientMessageId: "react-1",
      text: JSON.stringify({ messageId: "m1", emoji: "👍" }),
    });
    const due = listDuePending();
    assert.equal(due.length, 1);
    assert.equal(due[0]!.kind, "reaction_toggle");
    assert.equal(due[0]!.clientMessageId, "react-1");
    // Storage row id is outbox:${logicalOpId}; wire op is clientMessageId.
    assert.equal(due[0]!.id, "outbox:react-1");
  });

  it("includes approval_resolve in due queue with stable client op id (C5.3)", () => {
    const first = enqueueApprovalResolve({
      conversationId: "session-1",
      clientMessageId: "approval-op-1",
      text: JSON.stringify({
        requestId: "req-1",
        sessionId: "session-1",
        decision: { decision: "accept" },
      }),
    });
    assert.equal(first.kind, "approval_resolve");
    assert.equal(first.clientMessageId, "approval-op-1");
    assert.equal(first.id, "outbox:approval-op-1");
    assert.equal(listDuePending().length, 1);

    // Re-enqueue same logical op id keeps one entry (retry same id).
    const again = enqueueApprovalResolve({
      conversationId: "session-1",
      clientMessageId: "approval-op-1",
      text: JSON.stringify({
        requestId: "req-1",
        sessionId: "session-1",
        decision: { decision: "accept" },
      }),
    });
    assert.equal(again.clientMessageId, "approval-op-1");
    assert.equal(
      getOutboxSnapshotForTests().filter((e) => e.kind === "approval_resolve")
        .length,
      1,
    );
    assert.equal(listDuePending().length, 1);
    assert.equal(listDuePending()[0]!.clientMessageId, "approval-op-1");

    markInflight("approval-op-1");
    markAcked("approval-op-1");
    assert.equal(isAcked("approval-op-1"), true);
    assert.equal(listDuePending().length, 0);
  });
});
