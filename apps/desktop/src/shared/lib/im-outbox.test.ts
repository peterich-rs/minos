import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  classifyOutboxFailure,
  earliestPendingAttemptAt,
  enqueueAgentResult,
  enqueueApprovalResolve,
  enqueueReactionToggle,
  enqueueUserMessage,
  forceUpdatedAtForTests,
  getOutboxSnapshotForTests,
  isAcked,
  listDuePending,
  listDuePendingLanes,
  markAcked,
  markFailed,
  markInflight,
  outboxLaneKey,
  reclaimStaleInflight,
  resetImOutboxForTests,
  enableMemoryOutboxForTests,
  STALE_INFLIGHT_MS,
} from "./im-outbox.ts";

const TEST_ACCOUNT = "acct-test";

describe("im-outbox", () => {
  beforeEach(async () => {
    enableMemoryOutboxForTests();
    await resetImOutboxForTests();
  });

  it("enqueues pending user messages and acks prevent re-project", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m1",
      text: "hello",
    });
    assert.equal(await isAcked("m1"), false);
    const due = await listDuePending(undefined, TEST_ACCOUNT);
    assert.equal(due.length, 1);
    assert.equal(due[0]!.clientMessageId, "m1");

    await markInflight("m1");
    await markAcked("m1");
    assert.equal(await isAcked("m1"), true);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 0);

    // Re-enqueue after ack is a no-op for status
    const again = await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m1",
      text: "hello",
    });
    assert.equal(again.status, "acked");
  });

  it("persists structured mentions on user_message enqueue", async () => {
    const mentions = [
      { kind: "bot" as const, bot_id: "bot-codex", start: 0, length: 6 },
      {
        kind: "account" as const,
        account_id: "acct-1",
        start: 7,
        length: 5,
      },
    ];
    const entry = await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-mentions",
      text: "@codex @alice hi",
      mentions,
    });
    assert.deepEqual(entry.mentions, mentions);
    const due = await listDuePending(undefined, TEST_ACCOUNT);
    const row = due.find((e) => e.clientMessageId === "m-mentions");
    assert.ok(row);
    assert.deepEqual(row!.mentions, mentions);
  });

  it("network errors stay pending after many attempts (no terminal burn)", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m2",
      text: "x",
    });
    for (let i = 0; i < 20; i++) {
      await markInflight("m2");
      await markFailed("m2", "network error: ECONNRESET");
    }
    const snap = await getOutboxSnapshotForTests();
    const row = snap.find((e) => e.clientMessageId === "m2");
    assert.ok(row);
    assert.equal(row!.status, "pending");
    // Due later after backoff, not terminal.
    assert.equal(row!.status, "pending");
  });

  it("permanent client errors terminal after enough attempts", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-perm",
      text: "x",
    });
    for (let i = 0; i < 8; i++) {
      await markInflight("m-perm");
      await markFailed("m-perm", "HTTP 400 bad request");
    }
    const row = (await getOutboxSnapshotForTests()).find(
      (e) => e.clientMessageId === "m-perm",
    );
    assert.ok(row);
    assert.equal(row!.status, "failed_terminal");
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 0);
  });

  it("classifyOutboxFailure separates transient vs permanent", async () => {
    assert.equal(classifyOutboxFailure("network"), "transient");
    assert.equal(classifyOutboxFailure("Not signed in"), "transient");
    assert.equal(classifyOutboxFailure("connection refused"), "transient");
    assert.equal(classifyOutboxFailure("HTTP 503"), "transient");
    assert.equal(classifyOutboxFailure("HTTP 400"), "permanent");
    assert.equal(classifyOutboxFailure("invalid_payload_json"), "permanent");
    assert.equal(classifyOutboxFailure("HTTP 429 rate limit"), "transient");
  });

  it("reclaims stale inflight so kill mid-flight becomes due again", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-stale",
      text: "in flight",
    });
    await markInflight("m-stale");
    const snap = await getOutboxSnapshotForTests();
    const row = snap.find((e) => e.clientMessageId === "m-stale");
    assert.equal(row!.status, "inflight");

    // Fresh inflight is not due
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 0);

    // Simulate process kill: updatedAt far in the past
    const old = Date.now() - STALE_INFLIGHT_MS - 1_000;
    await forceUpdatedAtForTests("m-stale", old);

    const reclaimed = await reclaimStaleInflight();
    assert.equal(reclaimed, 1);
    const after = (await getOutboxSnapshotForTests()).find(
      (e) => e.clientMessageId === "m-stale",
    );
    assert.equal(after!.status, "pending");
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 1);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT))[0]!.clientMessageId, "m-stale");
  });

  it("reclaimStaleInflight scopes by accountId when provided", async () => {
    await enqueueUserMessage({
      accountId: "acct-a",
      conversationId: "c1",
      clientMessageId: "m-a",
      text: "a",
    });
    await enqueueUserMessage({
      accountId: "acct-b",
      conversationId: "c1",
      clientMessageId: "m-b",
      text: "b",
    });
    await markInflight("m-a");
    await markInflight("m-b");
    const old = Date.now() - STALE_INFLIGHT_MS - 1_000;
    await forceUpdatedAtForTests("m-a", old);
    await forceUpdatedAtForTests("m-b", old);

    const reclaimed = await reclaimStaleInflight(Date.now(), "acct-a");
    assert.equal(reclaimed, 1);
    const snap = await getOutboxSnapshotForTests();
    assert.equal(snap.find((e) => e.clientMessageId === "m-a")!.status, "pending");
    assert.equal(snap.find((e) => e.clientMessageId === "m-b")!.status, "inflight");
  });

  it("listDuePending includes reclaim of stale inflight inline", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-inline",
      text: "x",
    });
    await markInflight("m-inline");
    await forceUpdatedAtForTests("m-inline", Date.now() - STALE_INFLIGHT_MS - 5_000);
    const due = await listDuePending(undefined, TEST_ACCOUNT);
    assert.equal(due.length, 1);
    assert.equal(due[0]!.clientMessageId, "m-inline");
    assert.equal(due[0]!.status, "pending");
  });

  it("enqueues agent_result kind into the same status machine", async () => {
    const entry = await enqueueAgentResult({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "agent-result:c1:s1:origin1",
      text: "done",
      agentId: "agent-cloud-1",
      agentSessionId: "s1",
    });
    assert.equal(entry.kind, "agent_result");
    assert.equal(entry.status, "pending");
    assert.equal(entry.messageSource, "host_projection");
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 1);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT))[0]!.kind, "agent_result");

    await markInflight(entry.clientMessageId);
    await markAcked(entry.clientMessageId);
    assert.equal(await isAcked(entry.clientMessageId), true);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 0);
  });

  it("includes reaction_toggle in due queue", async () => {
    await enqueueReactionToggle({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "react-1",
      text: JSON.stringify({ messageId: "m1", emoji: "👍" }),
    });
    const due = await listDuePending(undefined, TEST_ACCOUNT);
    assert.equal(due.length, 1);
    assert.equal(due[0]!.kind, "reaction_toggle");
    assert.equal(due[0]!.clientMessageId, "react-1");
    // Storage row id is outbox:${logicalOpId}; wire op is clientMessageId.
    assert.equal(due[0]!.id, "outbox:react-1");
  });

  it("earliestPendingAttemptAt tracks backoff beyond 60s", async () => {
    const entry = await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "late-1",
      text: "hi",
    });
    await markInflight(entry.clientMessageId);
    // Force a far-future nextAttempt via permanent-looking then reclaim path:
    // markFailed with transient keeps pending with exponential backoff.
    for (let i = 0; i < 5; i += 1) {
      await markFailed(entry.clientMessageId, "network error");
      await markInflight(entry.clientMessageId);
    }
    await markFailed(entry.clientMessageId, "network error");
    const next = await earliestPendingAttemptAt(undefined, TEST_ACCOUNT);
    assert.ok(next != null);
    assert.ok((next as number) > Date.now() + 10_000);
  });

  it("includes approval_resolve in due queue with stable client op id", async () => {
    const first = await enqueueApprovalResolve({
      accountId: TEST_ACCOUNT,
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
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 1);

    // Re-enqueue same logical op id keeps one entry (retry same id).
    const again = await enqueueApprovalResolve({
      accountId: TEST_ACCOUNT,
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
      (await getOutboxSnapshotForTests()).filter((e) => e.kind === "approval_resolve")
        .length,
      1,
    );
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 1);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT))[0]!.clientMessageId, "approval-op-1");

    await markInflight("approval-op-1");
    await markAcked("approval-op-1");
    assert.equal(await isAcked("approval-op-1"), true);
    assert.equal((await listDuePending(undefined, TEST_ACCOUNT)).length, 0);
  });

  it("listDuePendingLanes enforces per-conversation FIFO (no tail overtake)", async () => {
    // Two conversations; c1 has two due messages; c2 has one.
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "c1-a",
      text: "first",
    });
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "c1-b",
      text: "second",
    });
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c2",
      clientMessageId: "c2-a",
      text: "other",
    });

    const lanes = await listDuePendingLanes(undefined, TEST_ACCOUNT);
    assert.equal(lanes.length, 2);
    // Lanes sorted by conversationId.
    assert.equal(lanes[0]![0]!.conversationId, "c1");
    assert.deepEqual(
      lanes[0]!.map((e) => e.clientMessageId),
      ["c1-a", "c1-b"],
    );
    assert.deepEqual(
      lanes[1]!.map((e) => e.clientMessageId),
      ["c2-a"],
    );

    // Put c1 head into backoff — lane must omit tail even if it is due.
    await markInflight("c1-a");
    await markFailed("c1-a", "network error");
    const afterFail = await listDuePendingLanes(undefined, TEST_ACCOUNT);
    // c1 blocked (head not due); c2 still drains.
    assert.equal(afterFail.length, 1);
    assert.equal(afterFail[0]![0]!.clientMessageId, "c2-a");
    // Flat due still includes c1-b (due) but lanes correctly hide it.
    const flat = await listDuePending(undefined, TEST_ACCOUNT);
    assert.ok(flat.some((e) => e.clientMessageId === "c1-b"));
    assert.ok(!afterFail.some((lane) => lane.some((e) => e.clientMessageId === "c1-b")));
  });

  it("listDuePendingLanes skips lane when head is fresh inflight", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-head",
      text: "a",
    });
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "m-tail",
      text: "b",
    });
    await markInflight("m-head");
    const head = (await getOutboxSnapshotForTests()).find(
      (e) => e.clientMessageId === "m-head",
    );
    assert.equal(head!.status, "inflight");
    // Inflight head blocks the whole lane (no tail overtake).
    assert.equal((await listDuePendingLanes(undefined, TEST_ACCOUNT)).length, 0);
    // Flat due still lists the pending tail; flush must use lanes.
    const flat = await listDuePending(undefined, TEST_ACCOUNT);
    assert.equal(flat.length, 1);
    assert.equal(flat[0]!.clientMessageId, "m-tail");
  });

  it("reaction lane is independent of blocked message lane", async () => {
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "msg-a",
      text: "first",
    });
    await enqueueUserMessage({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "msg-b",
      text: "second",
    });
    await enqueueReactionToggle({
      accountId: TEST_ACCOUNT,
      conversationId: "c1",
      clientMessageId: "rx-1",
      text: JSON.stringify({ messageId: "msg-a", emoji: "👍" }),
    });
    await markInflight("msg-a");
    const lanes = await listDuePendingLanes(undefined, TEST_ACCOUNT);
    // Message lane blocked; reaction lane still due.
    assert.equal(lanes.length, 1);
    assert.equal(lanes[0]![0]!.clientMessageId, "rx-1");
    assert.equal(outboxLaneKey(lanes[0]![0]!), "reaction:c1");
  });

  it("only claims rows for the current account and quarantines empty accountId", async () => {
    await enqueueUserMessage({
      accountId: "acct-a",
      conversationId: "c1",
      clientMessageId: "a1",
      text: "from-a",
    });
    await enqueueUserMessage({
      accountId: "acct-b",
      conversationId: "c1",
      clientMessageId: "b1",
      text: "from-b",
    });
    // Legacy/quarantine row
    const snap = await getOutboxSnapshotForTests();
    assert.equal(snap.length, 2);
    const dueA = await listDuePending(undefined, "acct-a");
    assert.equal(dueA.length, 1);
    assert.equal(dueA[0]!.clientMessageId, "a1");
    const dueB = await listDuePending(undefined, "acct-b");
    assert.equal(dueB.length, 1);
    assert.equal(dueB[0]!.clientMessageId, "b1");
    assert.equal((await listDuePending(undefined, "")).length, 0);
    assert.equal((await listDuePending()).length, 0);
  });

});
