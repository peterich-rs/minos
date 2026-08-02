import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { hubClientWsUrl } from "./minos-cloud.ts";
import {
  advanceTopicCursor,
  resumeAfterFromCursors,
  conversationTopic,
} from "./hub-cursors.ts";

describe("hubClientWsUrl", () => {
  it("resolves relative gateway path against backend base", () => {
    // backendHttpBase defaults to 127.0.0.1:8787 in tests without env.
    const url = hubClientWsUrl("/ws/client?ticket=abc", "abc");
    assert.match(url, /^ws:\/\//);
    assert.match(url, /\/ws\/client\?ticket=abc/);
  });

  it("appends ticket when missing", () => {
    const url = hubClientWsUrl("ws://127.0.0.1:8787/ws/client", "tok-1");
    assert.equal(url, "ws://127.0.0.1:8787/ws/client?ticket=tok-1");
  });
});

describe("hub-realtime cursor wiring helpers", () => {
  it("builds conversation resume_after after durable advances", () => {
    const topic = conversationTopic("conv-9");
    let cursors = advanceTopicCursor({}, topic, 12);
    cursors = advanceTopicCursor(cursors, "account:me", 4);
    const resume = resumeAfterFromCursors(cursors, [topic]);
    assert.deepEqual(resume, { [topic]: 12 });
  });
});
