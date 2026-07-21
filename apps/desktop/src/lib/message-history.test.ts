import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "./mock-data.ts";
import {
  firstMessageSeq,
  mergeMessagesOlder,
  mergeMessagesQuietTail,
  metaAfterMessageTail,
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
});
