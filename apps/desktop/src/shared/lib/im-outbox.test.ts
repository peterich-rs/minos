import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  enqueueUserMessage,
  getOutboxSnapshotForTests,
  isAcked,
  listDuePending,
  markAcked,
  markFailed,
  markInflight,
  resetImOutboxForTests,
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

  it("marks terminal failure after enough attempts", () => {
    enqueueUserMessage({
      conversationId: "c1",
      clientMessageId: "m2",
      text: "x",
    });
    for (let i = 0; i < 8; i++) {
      markInflight("m2");
      markFailed("m2", "network");
    }
    const snap = getOutboxSnapshotForTests();
    const row = snap.find((e) => e.clientMessageId === "m2");
    assert.ok(row);
    assert.equal(row!.status, "failed_terminal");
    assert.equal(listDuePending().length, 0);
  });
});
