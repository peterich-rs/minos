import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { sortTimelineMessages } from "./timeline-order.ts";
import type { TimelineMessage } from "./mock-data.ts";

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

describe("sortTimelineMessages", () => {
  it("orders durable messages by messageSeq ASC", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "c", messageSeq: 30, body: "c" }),
      msg({ id: "a", messageSeq: 10, body: "a" }),
      msg({ id: "b", messageSeq: 20, body: "b" }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["a", "b", "c"],
    );
  });

  it("keeps sending rows after earlier durable by createdAtMs when mixed seq", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "pending", deliveryStatus: "sending", createdAtMs: 999 }),
      msg({ id: "durable", messageSeq: 5, createdAtMs: 1 }),
    ]);
    assert.equal(sorted[0]?.id, "durable");
    assert.equal(sorted[1]?.id, "pending");
  });

  it("does not reorder by wall clock when both have seq", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "late-clock", messageSeq: 1, createdAtMs: 9_000 }),
      msg({ id: "early-clock", messageSeq: 2, createdAtMs: 1 }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["late-clock", "early-clock"],
    );
  });

  it("does not put seq-bearing agent above hub user without seq (same second)", () => {
    // Regression: Linked merge — Hub user has no messageSeq; local agent-result
    // has seq. Old sort put all seq rows before no-seq → agent above user.
    const sorted = sortTimelineMessages([
      msg({
        id: "agent-result:c:s:1",
        role: "agent",
        messageSeq: 12,
        createdAtMs: 1_700_000_100,
        body: "hi from grok",
      }),
      msg({
        id: "user-hub",
        role: "user",
        // no messageSeq (Hub projection)
        createdAtMs: 1_700_000_050,
        body: "@grok 你好",
      }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["user-hub", "agent-result:c:s:1"],
    );
  });

  it("orders user then agent when both have seq", () => {
    const sorted = sortTimelineMessages([
      msg({
        id: "agent-result:c:s:1",
        role: "agent",
        messageSeq: 2,
        createdAtMs: 200,
      }),
      msg({
        id: "user-1",
        role: "user",
        messageSeq: 1,
        createdAtMs: 100,
      }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["user-1", "agent-result:c:s:1"],
    );
  });
});
