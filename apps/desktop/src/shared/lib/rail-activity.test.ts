import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "../domain/collaboration.ts";
import {
  positiveMs,
  railActivityFromTimeline,
  resolveDigestLastActivityMs,
} from "./rail-activity.ts";

function msg(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id" | "body">,
): TimelineMessage {
  return {
    role: "user",
    time: "",
    createdAtMs: 0,
    ...partial,
  };
}

describe("railActivityFromTimeline", () => {
  it("returns null for empty / tool-only windows", () => {
    assert.equal(railActivityFromTimeline([]), null);
    assert.equal(
      railActivityFromTimeline([
        msg({
          id: "t1",
          body: "read foo",
          kind: "tool_summary",
          createdAtMs: 100,
        }),
      ]),
      null,
    );
  });

  it("picks the newest chat bubble from the tail", () => {
    const act = railActivityFromTimeline([
      msg({ id: "a", body: "old", createdAtMs: 100 }),
      msg({
        id: "t",
        body: "tool",
        kind: "tool_summary",
        createdAtMs: 200,
      }),
      msg({ id: "b", body: "newest", createdAtMs: 300 }),
    ]);
    assert.equal(act?.lastMessageAtMs, 300);
    assert.equal(act?.preview, "newest");
  });
});

describe("resolveDigestLastActivityMs", () => {
  it("append is monotonic max and never invents now", () => {
    assert.equal(
      resolveDigestLastActivityMs({
        isRecall: false,
        incomingLastAtMs: 50,
        previousLastMessageAtMs: 100,
      }),
      100,
    );
    assert.equal(
      resolveDigestLastActivityMs({
        isRecall: false,
        incomingLastAtMs: 200,
        previousLastMessageAtMs: 100,
      }),
      200,
    );
    assert.equal(
      resolveDigestLastActivityMs({
        isRecall: false,
        incomingLastAtMs: 0,
        previousLastMessageAtMs: 100,
      }),
      100,
    );
  });

  it("recall does not apply recalled message createdAtMs (would regress)", () => {
    // Timeline already dropped the recalled row; remaining newest is 500.
    const ms = resolveDigestLastActivityMs({
      isRecall: true,
      incomingLastAtMs: 100, // recalled bubble's original time — must be ignored
      previousLastMessageAtMs: 900,
      timeline: [
        msg({ id: "keep", body: "still here", createdAtMs: 500 }),
      ],
    });
    assert.equal(ms, 500);
  });

  it("recall without open window keeps previous last activity", () => {
    assert.equal(
      resolveDigestLastActivityMs({
        isRecall: true,
        incomingLastAtMs: 10,
        previousLastMessageAtMs: 777,
        timeline: undefined,
      }),
      777,
    );
  });
});

describe("positiveMs", () => {
  it("treats 0 / NaN as missing", () => {
    assert.equal(positiveMs(0), 0);
    assert.equal(positiveMs(undefined), 0);
    assert.equal(positiveMs(42), 42);
  });
});
