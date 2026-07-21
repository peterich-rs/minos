import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  reuseStableById,
  timelineMessageEqual,
  transcriptItemEqual,
} from "./list-identity.ts";

describe("reuseStableById", () => {
  it("returns prev array when content and order match", () => {
    const prev = [
      { id: "a", body: "1" },
      { id: "b", body: "2" },
    ];
    const next = [
      { id: "a", body: "1" },
      { id: "b", body: "2" },
    ];
    const out = reuseStableById(prev, next, (x, y) => x.body === y.body);
    assert.equal(out, prev);
    assert.equal(out[0], prev[0]);
    assert.equal(out[1], prev[1]);
  });

  it("reuses unchanged rows and replaces changed ones", () => {
    const a = { id: "a", body: "1" };
    const b = { id: "b", body: "2" };
    const prev = [a, b];
    const next = [
      { id: "a", body: "1" },
      { id: "b", body: "changed" },
      { id: "c", body: "3" },
    ];
    const out = reuseStableById(prev, next, (x, y) => x.body === y.body);
    assert.notEqual(out, prev);
    assert.equal(out[0], a);
    assert.notEqual(out[1], b);
    assert.equal(out[1]!.body, "changed");
    assert.equal(out[2]!.id, "c");
  });

  it("detects reorder even when contents match", () => {
    const a = { id: "a", body: "1" };
    const b = { id: "b", body: "2" };
    const prev = [a, b];
    const next = [
      { id: "b", body: "2" },
      { id: "a", body: "1" },
    ];
    const out = reuseStableById(prev, next, (x, y) => x.body === y.body);
    assert.notEqual(out, prev);
    assert.equal(out[0], b);
    assert.equal(out[1], a);
  });
});

describe("timelineMessageEqual", () => {
  it("matches on render-relevant fields", () => {
    const base = {
      id: "m1",
      role: "user" as const,
      body: "hi",
      time: "10:00",
      messageSeq: 1,
    };
    assert.equal(timelineMessageEqual(base, { ...base }), true);
    assert.equal(
      timelineMessageEqual(base, { ...base, body: "bye" }),
      false,
    );
  });
});

describe("transcriptItemEqual", () => {
  it("matches on render-relevant fields", () => {
    const base = {
      id: "t1",
      kind: "assistant",
      role: "assistant" as string | null,
      text: "hello",
      tsMs: 1,
      seq: 1,
    };
    assert.equal(transcriptItemEqual(base, { ...base }), true);
    assert.equal(
      transcriptItemEqual(base, { ...base, text: "hello!" }),
      false,
    );
  });
});
