import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  itemShowsStreamingCursor,
  streamingTailItemId,
} from "./transcript-streaming.ts";

describe("streamingTailItemId", () => {
  it("returns the tail when it is streamable text", () => {
    const id = streamingTailItemId([
      { id: "u1", kind: "user" },
      { id: "a1", kind: "assistant" },
    ]);
    assert.equal(id, "a1");
  });

  it("does not put a caret on a trailing user bubble while running", () => {
    assert.equal(
      streamingTailItemId([{ id: "u1", kind: "user" }]),
      null,
    );
  });

  it("returns null when tools follow intermediate narration", () => {
    // OpenCode-style: assistant text → task tool → subagent status while
    // session stays running. Cursor must not stick on the finished text.
    const id = streamingTailItemId([
      { id: "a1", kind: "assistant" },
      { id: "t1", kind: "tool" },
      { id: "s1", kind: "status" },
    ]);
    assert.equal(id, null);
  });

  it("returns null for empty transcript", () => {
    assert.equal(streamingTailItemId([]), null);
  });

  it("returns reasoning when that is the live tail", () => {
    assert.equal(
      streamingTailItemId([{ id: "r1", kind: "reasoning" }]),
      "r1",
    );
  });
});

describe("itemShowsStreamingCursor", () => {
  it("shows cursor only on live streamable tail", () => {
    assert.equal(
      itemShowsStreamingCursor(
        { id: "a1", kind: "assistant" },
        { sessionLive: true, streamingTailId: "a1" },
      ),
      true,
    );
  });

  it("hides cursor when session is idle even if tail is text", () => {
    assert.equal(
      itemShowsStreamingCursor(
        { id: "a1", kind: "assistant" },
        { sessionLive: false, streamingTailId: "a1" },
      ),
      false,
    );
  });

  it("hides cursor on earlier assistant while tools run", () => {
    assert.equal(
      itemShowsStreamingCursor(
        { id: "a1", kind: "assistant" },
        { sessionLive: true, streamingTailId: null },
      ),
      false,
    );
  });
});
