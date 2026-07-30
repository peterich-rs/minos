import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import {
  buildVirtualTimelineItems,
  estimateVirtualTimelineItemSize,
} from "./virtual-timeline-items.ts";

function msg(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id" | "body">,
): TimelineMessage {
  return {
    role: "user",
    agent: null,
    sessionId: null,
    time: "10:00",
    createdAtMs: 1_700_000_000_000,
    kind: "text",
    messageSeq: 1,
    ...partial,
  } as TimelineMessage;
}

describe("buildVirtualTimelineItems", () => {
  it("inserts a day divider before the first dated message", () => {
    const items = buildVirtualTimelineItems([
      msg({ id: "a", body: "hi", createdAtMs: Date.UTC(2026, 6, 22, 12) }),
    ]);
    assert.equal(items[0]?.type, "day");
    assert.equal(items[1]?.type, "message");
    assert.equal(items[1]?.type === "message" && items[1].id, "a");
  });

  it("marks same-author continuations", () => {
    const t0 = Date.UTC(2026, 6, 22, 12, 0);
    const items = buildVirtualTimelineItems([
      msg({ id: "a", body: "one", role: "user", createdAtMs: t0 }),
      msg({
        id: "b",
        body: "two",
        role: "user",
        createdAtMs: t0 + 60_000,
      }),
    ]);
    const messages = items.filter((i) => i.type === "message");
    assert.equal(messages[0]!.groupedWithPrevious, false);
    assert.equal(messages[1]!.groupedWithPrevious, true);
  });
});

describe("buildVirtualTimelineItems unread divider", () => {
  it("inserts unread divider after the read frontier", () => {
    const t0 = Date.UTC(2026, 6, 22, 12, 0);
    const items = buildVirtualTimelineItems(
      [
        msg({ id: "a", body: "one", messageSeq: 1, createdAtMs: t0 }),
        msg({ id: "b", body: "two", messageSeq: 2, createdAtMs: t0 + 1000 }),
        msg({ id: "c", body: "three", messageSeq: 3, createdAtMs: t0 + 2000 }),
      ],
      {
        readCursor: {
          readMessageCount: 1,
          lastReadSeq: 1,
          lastReadMessageId: "a",
          updatedAtMs: 1,
        },
      },
    );
    const types = items.map((i) => i.type);
    assert.ok(types.includes("unread"));
    const unreadIdx = types.indexOf("unread");
    // After day divider for first msg, message a, then unread, then b/c
    assert.equal(items[unreadIdx + 1]?.type, "message");
    assert.equal(
      items[unreadIdx + 1]?.type === "message" && items[unreadIdx + 1].id,
      "b",
    );
  });
});

describe("estimateVirtualTimelineItemSize", () => {
  it("returns a small size for day rows", () => {
    assert.equal(
      estimateVirtualTimelineItemSize({ type: "day", id: "d", ms: 1 }),
      36,
    );
  });

  it("returns a size for unread divider", () => {
    assert.equal(
      estimateVirtualTimelineItemSize({ type: "unread", id: "u" }),
      40,
    );
  });
});
