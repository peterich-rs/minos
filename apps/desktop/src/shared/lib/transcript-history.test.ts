import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  hasTranscriptWorkingSet,
  mergeTranscriptOlder,
  metaAfterTailLoad,
  olderPageRange,
  tailFromSeq,
  TRANSCRIPT_HARD_MAX_ITEMS,
  trimTranscriptHardMax,
} from "./transcript-history.ts";
import type { TranscriptItem } from "./daemon.ts";

function item(
  partial: Partial<TranscriptItem> & Pick<TranscriptItem, "id" | "seq" | "kind">,
): TranscriptItem {
  return {
    role: null,
    text: "",
    tsMs: 0,
    ...partial,
  };
}

describe("hasTranscriptWorkingSet", () => {
  it("false when key missing (must not create via ?? [])", () => {
    assert.equal(hasTranscriptWorkingSet({}, "s1"), false);
    assert.equal(hasTranscriptWorkingSet({ s2: [] }, "s1"), false);
  });

  it("true when key exists even if empty array", () => {
    assert.equal(hasTranscriptWorkingSet({ s1: [] }, "s1"), true);
    assert.equal(
      hasTranscriptWorkingSet(
        { s1: [item({ id: "a", seq: 1, kind: "user" })] },
        "s1",
      ),
      true,
    );
  });
});

describe("tailFromSeq", () => {
  it("undefined when history fits in window", () => {
    assert.equal(tailFromSeq(100, 400), undefined);
    assert.equal(tailFromSeq(400, 400), undefined);
  });

  it("seeks so the window ends at lastSeq", () => {
    // last=1999, window=400 → start=1600 → from=1599
    assert.equal(tailFromSeq(1999, 400), 1599);
  });
});

describe("olderPageRange", () => {
  it("null when already at start", () => {
    assert.equal(olderPageRange(1), null);
  });

  it("pages backward before firstLoadedStartSeq", () => {
    const r = olderPageRange(1600, 400);
    assert.ok(r);
    // end=1599, start=1200, from=1199, limit=400
    assert.equal(r!.fromSeq, 1199);
    assert.equal(r!.limit, 400);
    assert.equal(r!.nextFirstLoadedStartSeq, 1200);
  });

  it("clamps the first page to seq 1", () => {
    const r = olderPageRange(100, 400);
    assert.ok(r);
    assert.equal(r!.fromSeq, 0);
    assert.equal(r!.limit, 99);
    assert.equal(r!.nextFirstLoadedStartSeq, 1);
  });
});

describe("mergeTranscriptOlder", () => {
  it("prepends and merges split assistant chunks", () => {
    const older = [
      item({
        id: "o1",
        kind: "assistant",
        seq: 10,
        text: "hel",
        messageId: "m1",
      }),
    ];
    const newer = [
      item({
        id: "n1",
        kind: "assistant",
        seq: 20,
        text: "lo",
        messageId: "m1",
      }),
      item({ id: "n2", kind: "tool", seq: 21, text: "x" }),
    ];
    const out = mergeTranscriptOlder(older, newer);
    assert.equal(out.length, 2);
    assert.equal(out[0]!.text, "hello");
    assert.equal(out[0]!.seq, 20);
    assert.equal(out[1]!.id, "n2");
  });

  it("dedupes by id", () => {
    const a = item({ id: "same", kind: "user", seq: 1, text: "a" });
    const b = item({ id: "same", kind: "user", seq: 1, text: "a" });
    assert.deepEqual(mergeTranscriptOlder([a], [b]), [b]);
  });
});

describe("metaAfterTailLoad", () => {
  it("marks hasOlder when tail sought past start", () => {
    const m = metaAfterTailLoad(1599);
    assert.equal(m.firstLoadedStartSeq, 1600);
    assert.equal(m.hasOlder, true);
  });

  it("no older when started at 1", () => {
    const m = metaAfterTailLoad(undefined);
    assert.equal(m.firstLoadedStartSeq, 1);
    assert.equal(m.hasOlder, false);
  });
});

describe("trimTranscriptHardMax", () => {
  it("keeps all when under cap", () => {
    const items = [item({ id: "a", seq: 1, kind: "user" })];
    const out = trimTranscriptHardMax(items, 10);
    assert.equal(out.trimmed, false);
    assert.equal(out.items.length, 1);
  });

  it("drops oldest and reports trimmed", () => {
    const items = [
      item({ id: "a", seq: 1, kind: "user" }),
      item({ id: "b", seq: 2, kind: "assistant" }),
      item({ id: "c", seq: 3, kind: "tool" }),
    ];
    const out = trimTranscriptHardMax(items, 2);
    assert.equal(out.trimmed, true);
    assert.deepEqual(
      out.items.map((i) => i.id),
      ["b", "c"],
    );
  });

  it("default hardMax is TRANSCRIPT_HARD_MAX_ITEMS", () => {
    assert.equal(TRANSCRIPT_HARD_MAX_ITEMS, 2000);
  });
});
