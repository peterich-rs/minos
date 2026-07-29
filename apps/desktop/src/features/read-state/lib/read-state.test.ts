import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  advanceReadCursor,
  firstUnreadMessageIndex,
  latestMessageFrontier,
  migrateReadCursors,
  seedReadCursorIfAbsent,
  unreadCountFromCursor,
  type ConversationReadCursor,
} from "./read-state.ts";

describe("migrateReadCursors", () => {
  it("migrates legacy count map", () => {
    const out = migrateReadCursors({ a: 3, b: 0 }, 1000);
    assert.equal(out.a?.readMessageCount, 3);
    assert.equal(out.a?.updatedAtMs, 1000);
    assert.equal(out.b?.readMessageCount, 0);
  });

  it("pass-through cursor map", () => {
    const prev: ConversationReadCursor = {
      readMessageCount: 5,
      lastReadMessageId: "m1",
      lastReadSeq: 5,
      updatedAtMs: 50,
    };
    const out = migrateReadCursors({ c: prev });
    assert.deepEqual(out.c, prev);
  });

  it("ignores junk", () => {
    const out = migrateReadCursors({ x: "nope", y: null, z: {} });
    assert.deepEqual(out, {});
  });
});

describe("unreadCountFromCursor", () => {
  const cursor: ConversationReadCursor = {
    readMessageCount: 10,
    updatedAtMs: 1,
  };

  it("is zero when focused", () => {
    assert.equal(unreadCountFromCursor(20, cursor, true), 0);
  });

  it("is zero without cursor (first-sight policy)", () => {
    assert.equal(unreadCountFromCursor(20, undefined, false), 0);
  });

  it("counts growth past baseline", () => {
    assert.equal(unreadCountFromCursor(14, cursor, false), 4);
    assert.equal(unreadCountFromCursor(8, cursor, false), 0);
  });
});

describe("advanceReadCursor", () => {
  it("never rewinds count or seq", () => {
    const prev: ConversationReadCursor = {
      readMessageCount: 10,
      lastReadSeq: 10,
      lastReadMessageId: "m10",
      updatedAtMs: 1,
    };
    const next = advanceReadCursor(prev, {
      messageCount: 5,
      lastReadSeq: 3,
      lastReadMessageId: "m3",
      nowMs: 2,
    });
    assert.equal(next.readMessageCount, 10);
    assert.equal(next.lastReadSeq, 10);
    assert.equal(next.lastReadMessageId, "m10");
  });

  it("advances frontiers forward", () => {
    const prev: ConversationReadCursor = {
      readMessageCount: 2,
      lastReadSeq: 2,
      lastReadMessageId: "m2",
      updatedAtMs: 1,
    };
    const next = advanceReadCursor(prev, {
      messageCount: 5,
      lastReadSeq: 5,
      lastReadMessageId: "m5",
      nowMs: 9,
    });
    assert.equal(next.readMessageCount, 5);
    assert.equal(next.lastReadSeq, 5);
    assert.equal(next.lastReadMessageId, "m5");
    assert.equal(next.updatedAtMs, 9);
  });
});

describe("seedReadCursorIfAbsent", () => {
  it("seeds missing only", () => {
    const base = seedReadCursorIfAbsent({}, "a", 7, 1);
    assert.equal(base.a?.readMessageCount, 7);
    const again = seedReadCursorIfAbsent(base, "a", 99, 2);
    assert.equal(again.a?.readMessageCount, 7);
  });
});

describe("latestMessageFrontier / firstUnreadMessageIndex", () => {
  const messages = [
    { id: "a", messageSeq: 1 },
    { id: "b", messageSeq: 2 },
    { id: "c", messageSeq: 3 },
  ];

  it("picks highest seq", () => {
    assert.deepEqual(latestMessageFrontier(messages), {
      lastReadMessageId: "c",
      lastReadSeq: 3,
    });
  });

  it("finds first unread by seq", () => {
    assert.equal(
      firstUnreadMessageIndex(messages, {
        readMessageCount: 2,
        lastReadSeq: 2,
        updatedAtMs: 1,
      }),
      2,
    );
    assert.equal(
      firstUnreadMessageIndex(messages, {
        readMessageCount: 3,
        lastReadSeq: 3,
        updatedAtMs: 1,
      }),
      -1,
    );
  });

  it("falls back to id", () => {
    assert.equal(
      firstUnreadMessageIndex(messages, {
        readMessageCount: 1,
        lastReadMessageId: "a",
        updatedAtMs: 1,
      }),
      1,
    );
  });
});
