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

  it("keeps pending rows after durable seq when mixed", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "pending", pending: true, createdAtMs: 999 }),
      msg({ id: "durable", messageSeq: 5, createdAtMs: 1 }),
    ]);
    assert.equal(sorted[0]?.id, "durable");
    assert.equal(sorted[1]?.id, "pending");
  });

  it("does not reorder by wall clock when seq is present", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "late-clock", messageSeq: 1, createdAtMs: 9_000 }),
      msg({ id: "early-clock", messageSeq: 2, createdAtMs: 1 }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["late-clock", "early-clock"],
    );
  });
});
