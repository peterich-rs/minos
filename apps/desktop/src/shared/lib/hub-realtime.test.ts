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

describe("hub-realtime conversation subscription LRU (R4)", () => {
  it("evicts oldest when over cap", async () => {
    const {
      conversationSubscriptionLruTouch,
      MAX_OPEN_CONVERSATION_SUBSCRIPTIONS,
    } = await import("./conversation-sub-lru.ts");
    let ordered: string[] = [];
    for (let i = 0; i < MAX_OPEN_CONVERSATION_SUBSCRIPTIONS + 3; i++) {
      const r = conversationSubscriptionLruTouch(ordered, `conv-${i}`);
      ordered = r.next;
      if (i < MAX_OPEN_CONVERSATION_SUBSCRIPTIONS) {
        assert.equal(r.evicted.length, 0);
      } else {
        assert.equal(r.evicted.length, 1);
      }
    }
    assert.equal(ordered.length, MAX_OPEN_CONVERSATION_SUBSCRIPTIONS);
    assert.equal(ordered[0], "conv-3");
    assert.equal(
      ordered[ordered.length - 1],
      `conv-${MAX_OPEN_CONVERSATION_SUBSCRIPTIONS + 2}`,
    );
  });

  it("re-touch moves id to most-recent without growth", async () => {
    const { conversationSubscriptionLruTouch } = await import(
      "./conversation-sub-lru.ts"
    );
    let ordered = ["a", "b", "c"];
    const r = conversationSubscriptionLruTouch(ordered, "a", 3);
    assert.deepEqual(r.next, ["b", "c", "a"]);
    assert.deepEqual(r.evicted, []);
  });
});
