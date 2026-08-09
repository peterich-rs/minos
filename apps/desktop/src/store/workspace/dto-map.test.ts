/**
 * Pure status generation policy (statusForLoad / bumpStatus).
 * Duplicated helpers — node:test cannot resolve @/ imports from dto-map.ts.
 * Keep in sync with store/workspace/dto-map.ts.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

type ResourceFetchPhase = "idle" | "loading" | "ready" | "error";
type ResourceFetchStatus = {
  phase: ResourceFetchPhase;
  generation: number;
  error?: string;
};

function bumpStatus(
  prev: ResourceFetchStatus | undefined,
  quiet: boolean,
): { next: ResourceFetchStatus; generation: number } {
  const generation = (prev?.generation ?? 0) + 1;
  const phase: ResourceFetchPhase =
    quiet && prev?.phase === "ready" ? "ready" : "loading";
  return {
    generation,
    next: { phase, generation, error: undefined },
  };
}

/** Mirrors dto-map.statusForLoad — quiet reuses gen; hard bumps. */
function statusForLoad(
  prev: ResourceFetchStatus | undefined,
  quiet: boolean,
): { next: ResourceFetchStatus; generation: number } {
  if (quiet) {
    let generation = prev?.generation ?? 0;
    const next: ResourceFetchStatus =
      prev?.phase === "ready"
        ? { phase: "ready", generation }
        : { phase: "loading", generation: Math.max(generation, 1) };
    if (next.generation !== generation) {
      generation = next.generation;
    }
    return { next, generation };
  }
  const bumped = bumpStatus(prev, false);
  return {
    generation: bumped.generation,
    next: { phase: "loading", generation: bumped.generation },
  };
}

describe("statusForLoad", () => {
  it("reuses generation on quiet when already ready", () => {
    const prev: ResourceFetchStatus = { phase: "ready", generation: 3 };
    const { next, generation } = statusForLoad(prev, true);
    assert.equal(generation, 3);
    assert.equal(next.phase, "ready");
    assert.equal(next.generation, 3);
  });

  it("does not bump generation on quiet when loading", () => {
    const prev: ResourceFetchStatus = { phase: "loading", generation: 2 };
    const { next, generation } = statusForLoad(prev, true);
    assert.equal(generation, 2);
    assert.equal(next.phase, "loading");
    assert.equal(next.generation, 2);
  });

  it("seeds generation 1 on first quiet with no prev", () => {
    const { next, generation } = statusForLoad(undefined, true);
    assert.equal(generation, 1);
    assert.equal(next.phase, "loading");
    assert.equal(next.generation, 1);
  });

  it("bumps generation on hard open", () => {
    const prev: ResourceFetchStatus = { phase: "ready", generation: 3 };
    const { next, generation } = statusForLoad(prev, false);
    assert.equal(generation, 4);
    assert.equal(next.phase, "loading");
    assert.equal(next.generation, 4);
  });

  it("quiet does not advance past a concurrent hard open generation", () => {
    const quietAfterReady = statusForLoad(
      { phase: "ready", generation: 5 },
      true,
    );
    assert.equal(quietAfterReady.generation, 5);
    assert.equal(quietAfterReady.next.phase, "ready");

    const hard = statusForLoad(
      { phase: "loading", generation: 5 },
      false,
    );
    assert.equal(hard.generation, 6);
  });
});

describe("bumpStatus", () => {
  it("always increments generation (legacy hard path)", () => {
    const prev: ResourceFetchStatus = { phase: "ready", generation: 1 };
    const quiet = bumpStatus(prev, true);
    assert.equal(quiet.generation, 2);
    assert.equal(quiet.next.phase, "ready");
    const hard = bumpStatus(prev, false);
    assert.equal(hard.generation, 2);
    assert.equal(hard.next.phase, "loading");
  });
});
