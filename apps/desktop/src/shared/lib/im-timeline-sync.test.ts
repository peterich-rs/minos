/**
 * TimelineSync pure helpers: max seq + quiet-tail merge used by
 * SnapshotRequired range reconcile (no clear-only).
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "./mock-data.ts";
import {
  firstMessageSeq,
  lastMessageSeq,
  mergeMessagesQuietTail,
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

describe("TimelineSync range reconcile helpers", () => {
  it("derives min/max loaded seq for after_seq / before_seq", () => {
    const window = [
      msg({ id: "1", messageSeq: 10 }),
      msg({ id: "2", messageSeq: 11 }),
      msg({ id: "opt", deliveryStatus: "sending" }),
    ];
    assert.equal(firstMessageSeq(window), 10);
    assert.equal(lastMessageSeq(window), 11);
  });

  it("quiet-tail merge keeps older pages across snapshot (no clear)", () => {
    const prev = [
      msg({ id: "old", messageSeq: 1, body: "older page" }),
      msg({ id: "mid", messageSeq: 5, body: "mid" }),
      msg({ id: "tail", messageSeq: 10, body: "stale tail" }),
    ];
    // Snapshot latest page + forward gap (no blank clear).
    const cloudChunk = [
      msg({ id: "mid", messageSeq: 5, body: "mid" }),
      msg({ id: "tail", messageSeq: 10, body: "fresh tail" }),
      msg({ id: "new", messageSeq: 11, body: "forward" }),
    ];
    const merged = mergeMessagesQuietTail(prev, cloudChunk);
    assert.deepEqual(
      merged.map((m) => m.id),
      ["old", "mid", "tail", "new"],
    );
    assert.equal(merged.find((m) => m.id === "tail")!.body, "fresh tail");
    // Older page retained — clear-only strategy would have dropped "old".
    assert.equal(merged.find((m) => m.id === "old")!.body, "older page");
  });
});
