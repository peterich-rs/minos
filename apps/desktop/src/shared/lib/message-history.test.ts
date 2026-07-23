import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "./mock-data.ts";
import {
  firstMessageSeq,
  hasTimelineWorkingSet,
  mergeMessagesOlder,
  mergeMessagesQuietTail,
  metaAfterMessageTail,
  TIMELINE_HARD_MAX_MESSAGES,
  trimMessagesHardMax,
} from "./message-history.ts";

function msg(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id">,
): TimelineMessage {
  return {
    role: "user",
    body: partial.body ?? partial.id,
    time: "now",
    ...partial,
  };
}

describe("firstMessageSeq / metaAfterMessageTail", () => {
  it("finds the lowest durable seq", () => {
    assert.equal(
      firstMessageSeq([
        msg({ id: "b", messageSeq: 20 }),
        msg({ id: "a", messageSeq: 10 }),
        msg({ id: "p" }),
      ]),
      10,
    );
  });

  it("records hasOlder from the daemon flag", () => {
    const meta = metaAfterMessageTail(
      [msg({ id: "a", messageSeq: 50 })],
      true,
    );
    assert.equal(meta.firstLoadedSeq, 50);
    assert.equal(meta.hasOlder, true);
    assert.equal(meta.loadingOlder, false);
  });
});

describe("mergeMessagesOlder", () => {
  it("prepends older rows and dedupes by id", () => {
    const older = [
      msg({ id: "1", messageSeq: 1, body: "a" }),
      msg({ id: "2", messageSeq: 2, body: "b" }),
    ];
    const newer = [
      msg({ id: "2", messageSeq: 2, body: "b" }),
      msg({ id: "3", messageSeq: 3, body: "c" }),
    ];
    const out = mergeMessagesOlder(older, newer);
    assert.deepEqual(
      out.map((m) => m.id),
      ["1", "2", "3"],
    );
  });
});

describe("hasTimelineWorkingSet", () => {
  it("false when key missing (must not create via ?? [])", () => {
    assert.equal(hasTimelineWorkingSet({}, "c1"), false);
    assert.equal(hasTimelineWorkingSet({ c2: [] }, "c1"), false);
  });

  it("true when messages key exists even if empty array", () => {
    assert.equal(hasTimelineWorkingSet({ c1: [] }, "c1"), true);
  });

  it("true when history meta or timeline status key exists", () => {
    assert.equal(
      hasTimelineWorkingSet({}, "c1", {
        messageHistoryByConversation: { c1: {} },
      }),
      true,
    );
    assert.equal(
      hasTimelineWorkingSet({}, "c1", {
        timelineStatusByConversation: { c1: {} },
      }),
      true,
    );
  });
});

describe("mergeMessagesQuietTail", () => {
  it("keeps older pages when tail re-list is partial", () => {
    const prev = [
      msg({ id: "1", messageSeq: 1, body: "old" }),
      msg({ id: "2", messageSeq: 2, body: "mid" }),
      msg({ id: "3", messageSeq: 3, body: "new" }),
    ];
    const tail = [
      msg({ id: "2", messageSeq: 2, body: "mid" }),
      msg({ id: "3", messageSeq: 3, body: "new+" }),
      msg({ id: "4", messageSeq: 4, body: "latest" }),
    ];
    const out = mergeMessagesQuietTail(prev, tail);
    assert.deepEqual(
      out.map((m) => m.id),
      ["1", "2", "3", "4"],
    );
    assert.equal(out.find((m) => m.id === "3")!.body, "new+");
    // Unchanged older row keeps identity when equal.
    assert.equal(out.find((m) => m.id === "1"), prev[0]);
  });

  it("reuses identity for unchanged tail rows", () => {
    const mid = msg({ id: "2", messageSeq: 2, body: "mid" });
    const prev = [msg({ id: "1", messageSeq: 1 }), mid];
    const tail = [
      msg({ id: "2", messageSeq: 2, body: "mid" }),
      msg({ id: "3", messageSeq: 3 }),
    ];
    const out = mergeMessagesQuietTail(prev, tail);
    assert.equal(out.find((m) => m.id === "2"), mid);
  });

  it("keeps concurrent newer rows when a stale tail page lands later", () => {
    // Open RPC started at T0; quiet re-list after append already has msg 3.
    // Hard open must union-merge, not replace with the stale T0 page.
    const prev = [
      msg({ id: "1", messageSeq: 1, body: "a" }),
      msg({ id: "2", messageSeq: 2, body: "b" }),
      msg({ id: "3", messageSeq: 3, body: "agent done" }),
    ];
    const staleTail = [
      msg({ id: "1", messageSeq: 1, body: "a" }),
      msg({ id: "2", messageSeq: 2, body: "b" }),
    ];
    const out = mergeMessagesQuietTail(prev, staleTail);
    assert.deepEqual(
      out.map((m) => m.id),
      ["1", "2", "3"],
    );
  });
});

describe("trimMessagesHardMax", () => {
  it("keeps all when under cap", () => {
    const msgs = [msg({ id: "1" }), msg({ id: "2" })];
    const out = trimMessagesHardMax(msgs, 10);
    assert.equal(out.trimmed, false);
    assert.equal(out.messages.length, 2);
  });

  it("drops oldest and reports trimmed", () => {
    const msgs = [
      msg({ id: "1", messageSeq: 1 }),
      msg({ id: "2", messageSeq: 2 }),
      msg({ id: "3", messageSeq: 3 }),
      msg({ id: "4", messageSeq: 4 }),
    ];
    const out = trimMessagesHardMax(msgs, 2);
    assert.equal(out.trimmed, true);
    assert.deepEqual(
      out.messages.map((m) => m.id),
      ["3", "4"],
    );
  });

  it("default hardMax is TIMELINE_HARD_MAX_MESSAGES", () => {
    assert.equal(TIMELINE_HARD_MAX_MESSAGES, 500);
  });
});
