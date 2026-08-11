import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "../domain/collaboration.ts";
import { sortTimelineMessages } from "./timeline-order.ts";

function msg(partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id">): TimelineMessage {
  return {
    role: "user",
    body: partial.body ?? partial.id,
    time: "now",
    ...partial,
  };
}

describe("sortTimelineMessages", () => {
  it("orders durable chat by messageSeq ASC (Hub social)", () => {
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

  it("keeps sending rows after durable by hub seq", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "pending", deliveryStatus: "sending", createdAtMs: 999 }),
      msg({ id: "durable", messageSeq: 5, createdAtMs: 1 }),
    ]);
    assert.equal(sorted[0]?.id, "durable");
    assert.equal(sorted[1]?.id, "pending");
  });

  it("does not reorder by wall clock when both have hub seq", () => {
    const sorted = sortTimelineMessages([
      msg({ id: "late-clock", messageSeq: 1, createdAtMs: 9_000 }),
      msg({ id: "early-clock", messageSeq: 2, createdAtMs: 1 }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["late-clock", "early-clock"],
    );
  });

  it("puts hub-seq durable before optimistic no-seq peers", () => {
    const sorted = sortTimelineMessages([
      msg({
        id: "pending",
        role: "user",
        deliveryStatus: "sending",
        createdAtMs: 1_700_000_050,
        body: "@grok 你好",
      }),
      msg({
        id: "agent-result:c:s:1",
        role: "agent",
        messageSeq: 12,
        createdAtMs: 1_700_000_100,
        body: "hi from grok",
      }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["agent-result:c:s:1", "pending"],
    );
  });

  it("orders user then agent when both have hub seq", () => {
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

  it("orders hub-only mobile bubble by hub seq among host-known peers", () => {
    // Hub SSOT: mobile-only row keeps its hub seq (not stripped to wall clock).
    const sorted = sortTimelineMessages([
      msg({
        id: "agent",
        role: "agent",
        messageSeq: 12,
        createdAtMs: 200,
      }),
      msg({
        id: "mobile",
        role: "user",
        messageSeq: 11,
        createdAtMs: 50,
      }),
      msg({
        id: "host-user",
        role: "user",
        messageSeq: 10,
        createdAtMs: 100,
      }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["host-user", "mobile", "agent"],
    );
  });

  it("places host tool cards after their hub anchor, not by wall clock", () => {
    const sorted = sortTimelineMessages([
      msg({
        id: "tool",
        role: "system",
        kind: "tool_summary",
        body: "rg",
        anchorCloudMessageSeq: 10,
        suborder: 1,
        createdAtMs: 1,
      }),
      msg({
        id: "user",
        role: "user",
        messageSeq: 10,
        createdAtMs: 100,
      }),
      msg({
        id: "agent",
        role: "agent",
        messageSeq: 11,
        createdAtMs: 200,
      }),
    ]);
    assert.deepEqual(
      sorted.map((m) => m.id),
      ["user", "tool", "agent"],
    );
  });

  it("is stable across input permutations for hub+anchor windows", () => {
    const rows = [
      msg({ id: "a", messageSeq: 1, createdAtMs: 100 }),
      msg({ id: "b", messageSeq: 2, createdAtMs: 10 }),
      msg({
        id: "tool",
        role: "system",
        kind: "tool_summary",
        body: "t",
        anchorCloudMessageSeq: 1,
        suborder: 1,
        createdAtMs: 50,
      }),
    ];
    const expected = ["a", "tool", "b"];
    const permutations = [
      [rows[0], rows[1], rows[2]],
      [rows[0], rows[2], rows[1]],
      [rows[1], rows[0], rows[2]],
      [rows[1], rows[2], rows[0]],
      [rows[2], rows[0], rows[1]],
      [rows[2], rows[1], rows[0]],
    ];
    for (const permutation of permutations) {
      assert.deepEqual(
        sortTimelineMessages(permutation as TimelineMessage[]).map((m) => m.id),
        expected,
      );
    }
  });
});
