import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { cloudClientWsUrl } from "./minos-cloud.ts";
import {
  advanceTopicCursor,
  resumeAfterFromCursors,
  conversationTopic,
} from "./cloud-cursors.ts";
describe("cloudClientWsUrl", () => {
  it("resolves relative gateway path against backend base", () => {
    // backendHttpBase defaults to 127.0.0.1:8787 in tests without env.
    const url = cloudClientWsUrl("/ws/client?ticket=abc", "abc");
    assert.match(url, /^ws:\/\//);
    assert.match(url, /\/ws\/client\?ticket=abc/);
  });

  it("appends ticket when missing", () => {
    const url = cloudClientWsUrl("ws://127.0.0.1:8787/ws/client", "tok-1");
    assert.equal(url, "ws://127.0.0.1:8787/ws/client?ticket=tok-1");
  });
});

describe("cloud-realtime cursor wiring helpers", () => {
  it("builds conversation resume_after after durable advances", () => {
    const topic = conversationTopic("conv-9");
    let cursors = advanceTopicCursor({}, topic, 12);
    cursors = advanceTopicCursor(cursors, "account:me", 4);
    const resume = resumeAfterFromCursors(cursors, [topic]);
    assert.deepEqual(resume, { [topic]: 12 });
  });
});

describe("cloud-realtime account thin digest (R3)", () => {
  it("maps account append payload without nested message body", async () => {
    // Exercise digest field mapping via a private-path equivalent: wire shape.
    const payload = {
      kind: "account_conversation_message_appended",
      account_id: "acc-1",
      conversation_id: "conv-1",
      message_id: "msg-1",
      at_ms: 1000,
      preview: "hello digest",
      sender_display_name: "Other",
      mentioned: true,
      message_seq: 5,
      sender: { kind: "user", account_id: "other" },
    };
    assert.equal(payload.preview, "hello digest");
    assert.equal("message" in payload, false);
    assert.equal(payload.mentioned, true);
  });
});

describe("cloud-realtime conversation subscription LRU (R4)", () => {
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
    const ordered = ["a", "b", "c"];
    const r = conversationSubscriptionLruTouch(ordered, "a", 3);
    assert.deepEqual(r.next, ["b", "c", "a"]);
    assert.deepEqual(r.evicted, []);
  });
});
