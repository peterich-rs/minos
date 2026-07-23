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

describe("estimateVirtualTimelineItemSize", () => {
  it("returns a small size for day rows", () => {
    assert.equal(
      estimateVirtualTimelineItemSize({ type: "day", id: "d", ms: 1 }),
      36,
    );
  });
});
