import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  advanceTopicCursor,
  clearTopicCursor,
  conversationTopic,
  accountTopic,
  resumeAfterFromCursors,
  loadTopicCursors,
  saveTopicCursors,
  HUB_CURSOR_STORAGE_KEY,
} from "./hub-cursors.ts";

describe("advanceTopicCursor", () => {
  it("inserts and advances monotonically", () => {
    let m = advanceTopicCursor({}, "conversation:c1", 3);
    assert.equal(m["conversation:c1"], 3);
    m = advanceTopicCursor(m, "conversation:c1", 5);
    assert.equal(m["conversation:c1"], 5);
    const same = advanceTopicCursor(m, "conversation:c1", 4);
    assert.equal(same, m);
    assert.equal(same["conversation:c1"], 5);
  });

  it("ignores invalid inputs", () => {
    const base = { "account:a": 1 };
    assert.equal(advanceTopicCursor(base, "", 2), base);
    assert.equal(advanceTopicCursor(base, "t", Number.NaN), base);
  });
});

describe("clearTopicCursor / resumeAfterFromCursors", () => {
  it("clears one topic and builds resume_after", () => {
    const m = { "conversation:c1": 10, "account:a": 2 };
    const cleared = clearTopicCursor(m, "conversation:c1");
    assert.equal(cleared["conversation:c1"], undefined);
    assert.equal(cleared["account:a"], 2);

    const resume = resumeAfterFromCursors(
      { "conversation:c1": 10, "conversation:c2": 0 },
      ["conversation:c1", "conversation:c2", "conversation:c3"],
    );
    assert.deepEqual(resume, { "conversation:c1": 10 });
    assert.equal(
      resumeAfterFromCursors({}, ["conversation:c1"]),
      undefined,
    );
  });
});

describe("topic helpers + storage", () => {
  it("formats topics", () => {
    assert.equal(conversationTopic("abc"), "conversation:abc");
    assert.equal(accountTopic("u1"), "account:u1");
  });

  it("round-trips through a fake storage", () => {
    const bag = new Map<string, string>();
    const storage = {
      getItem: (k: string) => bag.get(k) ?? null,
      setItem: (k: string, v: string) => {
        bag.set(k, v);
      },
    };
    saveTopicCursors({ "conversation:x": 7 }, storage);
    assert.ok(bag.has(HUB_CURSOR_STORAGE_KEY));
    const loaded = loadTopicCursors(storage);
    assert.equal(loaded["conversation:x"], 7);
  });
});
