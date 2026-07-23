import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  demoteResolvedApprovalItems,
  deriveSessionStatus,
  nextSessionStatusAfterTranscript,
  nextStatusFromManagerEvent,
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

describe("demoteResolvedApprovalItems", () => {
  it("demotes plan approval when later assistant progress exists", () => {
    const items = [
      {
        kind: "approval",
        seq: 10,
        requestId: "plan-1",
        approvalMethod: "x.ai/exit_plan_mode",
        text: "needs approval",
        title: "Plan approval",
      },
      {
        kind: "assistant",
        seq: 20,
        requestId: null,
        text: "计划已批准，开始实现",
        title: null,
      },
    ];
    const out = demoteResolvedApprovalItems(items);
    assert.equal(out[0]!.kind, "status");
    assert.equal(out[0]!.requestId, null);
    assert.equal(out[0]!.text, "Plan approved");
    assert.equal(transcriptHasPendingApproval(out), false);
  });

  it("keeps terminal pending approval when agent is still parked", () => {
    const items = [
      {
        kind: "tool",
        seq: 5,
        requestId: null,
        text: "read_file",
        title: null,
      },
      {
        kind: "approval",
        seq: 10,
        requestId: "plan-2",
        approvalMethod: "x.ai/exit_plan_mode",
        text: "needs approval",
        title: "Plan approval",
      },
    ];
    const out = demoteResolvedApprovalItems(items);
    assert.equal(out[1]!.kind, "approval");
    assert.equal(out[1]!.requestId, "plan-2");
    assert.equal(transcriptHasPendingApproval(out), true);
  });

  it("demotes only cards before later progress when a newer request is open", () => {
    const items = [
      {
        kind: "approval",
        seq: 10,
        requestId: "old",
        approvalMethod: "session/request_permission",
        text: "old perm",
        title: "Permission",
      },
      {
        kind: "tool",
        seq: 20,
        requestId: null,
        text: "bash",
        title: null,
      },
      {
        kind: "approval",
        seq: 30,
        requestId: "new",
        approvalMethod: "session/request_permission",
        text: "new perm",
        title: "Permission",
      },
    ];
    const out = demoteResolvedApprovalItems(items);
    assert.equal(out[0]!.requestId, null);
    assert.equal(out[0]!.kind, "status");
    assert.equal(out[2]!.requestId, "new");
    assert.equal(out[2]!.kind, "approval");
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

describe("nextStatusFromManagerEvent", () => {
  it("holds needs_approval only while daemon still says running", () => {
    assert.equal(
      nextStatusFromManagerEvent("needs_approval", "running"),
      "needs_approval",
    );
  });

  it("applies idle/done so ghost running cannot stick after turn ends", () => {
    assert.equal(nextStatusFromManagerEvent("needs_approval", "idle"), "idle");
    assert.equal(nextStatusFromManagerEvent("needs_approval", "done"), "done");
    assert.equal(
      nextStatusFromManagerEvent("needs_approval", "suspended"),
      "suspended",
    );
  });

  it("passes through normal running → idle / done transitions", () => {
    assert.equal(nextStatusFromManagerEvent("running", "idle"), "idle");
    assert.equal(nextStatusFromManagerEvent("running", "done"), "done");
    assert.equal(nextStatusFromManagerEvent("idle", "running"), "running");
  });
});
