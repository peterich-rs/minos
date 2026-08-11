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
  CLOUD_CURSOR_STORAGE_KEY,
  LEGACY_CLOUD_CURSOR_STORAGE_KEY,
} from "./cloud-cursors.ts";

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
      removeItem: (k: string) => {
        bag.delete(k);
      },
    };
    saveTopicCursors({ "conversation:x": 7 }, storage);
    assert.ok(bag.has(CLOUD_CURSOR_STORAGE_KEY));
    assert.equal(bag.has(LEGACY_CLOUD_CURSOR_STORAGE_KEY), false);
    const loaded = loadTopicCursors(storage);
    assert.equal(loaded["conversation:x"], 7);
  });

  it("migrates legacy hub cursor key to cloud key", () => {
    const bag = new Map<string, string>();
    bag.set(
      LEGACY_CLOUD_CURSOR_STORAGE_KEY,
      JSON.stringify({ "conversation:legacy": 42 }),
    );
    const storage = {
      getItem: (k: string) => bag.get(k) ?? null,
      setItem: (k: string, v: string) => {
        bag.set(k, v);
      },
      removeItem: (k: string) => {
        bag.delete(k);
      },
    };
    const loaded = loadTopicCursors(storage);
    assert.equal(loaded["conversation:legacy"], 42);
    assert.ok(bag.has(CLOUD_CURSOR_STORAGE_KEY));
    assert.equal(bag.has(LEGACY_CLOUD_CURSOR_STORAGE_KEY), false);
    assert.equal(
      JSON.parse(bag.get(CLOUD_CURSOR_STORAGE_KEY)! )["conversation:legacy"],
      42,
    );
  });
});
