import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  deriveSessionStatus,
  nextSessionStatusAfterTranscript,
  transcriptHasPendingApproval,
  withDerivedSessionStatuses,
} from "./session-status.ts"; // .ts required for node --experimental-strip-types

describe("deriveSessionStatus", () => {
  it("forces needs_approval when peek finds pending approval", () => {
    assert.equal(
      deriveSessionStatus("running", { pendingApproval: true }),
      "needs_approval",
    );
  });

  it("trusts daemon status when peek finds no pending approval", () => {
    assert.equal(
      deriveSessionStatus("running", {
        prevStatus: "needs_approval",
        pendingApproval: false,
      }),
      "running",
    );
    assert.equal(
      deriveSessionStatus("done", {
        prevStatus: "needs_approval",
        pendingApproval: false,
      }),
      "done",
    );
  });

  it("preserves needs_approval across poll when peek has not settled", () => {
    assert.equal(
      deriveSessionStatus("running", {
        prevStatus: "needs_approval",
        pendingApproval: undefined,
      }),
      "needs_approval",
    );
  });

  it("does not preserve needs_approval when daemon left running", () => {
    assert.equal(
      deriveSessionStatus("idle", {
        prevStatus: "needs_approval",
      }),
      "idle",
    );
    assert.equal(
      deriveSessionStatus("suspended", {
        prevStatus: "needs_approval",
      }),
      "suspended",
    );
  });

  it("passes through daemon status when no prior elevation", () => {
    assert.equal(deriveSessionStatus("running"), "running");
    assert.equal(deriveSessionStatus("done"), "done");
  });
});

describe("transcriptHasPendingApproval", () => {
  it("detects approval items with requestId", () => {
    assert.equal(
      transcriptHasPendingApproval([
        { kind: "text", requestId: null },
        { kind: "approval", requestId: "req-1" },
      ]),
      true,
    );
  });

  it("ignores approvals without requestId and non-approval kinds", () => {
    assert.equal(
      transcriptHasPendingApproval([
        { kind: "approval", requestId: null },
        { kind: "status", requestId: "x" },
      ]),
      false,
    );
  });
});

describe("withDerivedSessionStatuses", () => {
  it("holds needs_approval until pending map clears it", () => {
    const daemon = [
      { id: "a", status: "running" as const },
      { id: "b", status: "running" as const },
    ];
    const prev = new Map([
      ["a", "needs_approval" as const],
      ["b", "running" as const],
    ]);
    const held = withDerivedSessionStatuses(daemon, prev);
    assert.equal(held[0]!.status, "needs_approval");
    assert.equal(held[1]!.status, "running");

    const cleared = withDerivedSessionStatuses(
      daemon,
      prev,
      new Map([
        ["a", false],
        ["b", false],
      ]),
    );
    assert.equal(cleared[0]!.status, "running");
    assert.equal(cleared[1]!.status, "running");

    const pending = withDerivedSessionStatuses(
      daemon,
      prev,
      new Map([["a", true]]),
    );
    assert.equal(pending[0]!.status, "needs_approval");
  });

  it("missing pending key preserves elevation (does not treat as false)", () => {
    const daemon = [{ id: "a", status: "running" as const }];
    const prev = new Map([["a", "needs_approval" as const]]);
    // Map with only other ids — `a` must preserve, not clear.
    const out = withDerivedSessionStatuses(
      daemon,
      prev,
      new Map([["other", false]]),
    );
    assert.equal(out[0]!.status, "needs_approval");
  });
});

// mergeTranscriptItems lives in workspace-store; tested lightly here via re-export if needed.
// Keep session-status pure unit tests self-contained.

describe("nextSessionStatusAfterTranscript", () => {
  it("elevate-only never demotes needs_approval on empty peek", () => {
    assert.equal(
      nextSessionStatusAfterTranscript({
        current: "needs_approval",
        hasPendingApproval: false,
        policy: "elevate-only",
      }),
      "needs_approval",
    );
  });

  it("elevate-only still elevates when pending found", () => {
    assert.equal(
      nextSessionStatusAfterTranscript({
        current: "running",
        hasPendingApproval: true,
        policy: "elevate-only",
      }),
      "needs_approval",
    );
  });

  it("sync demotes when pending gone", () => {
    assert.equal(
      nextSessionStatusAfterTranscript({
        current: "needs_approval",
        hasPendingApproval: false,
        policy: "sync",
      }),
      "running",
    );
  });
});
