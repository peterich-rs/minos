import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { resolveDispatchTargets } from "./resolve-dispatch-targets.ts";
import type { MentionProfile } from "../../shared/lib/agent-route.ts";

const installed = new Set(["codex", "claude", "grok"]);
const profiles: MentionProfile[] = [
  { id: "profile-research", name: "ResearchGrok", runtimeAgent: "grok" },
];

describe("resolveDispatchTargets", () => {
  it("pure human: zero agents → empty targets (no throw)", () => {
    const r = resolveDispatchTargets({
      messageBody: "hey team, lunch?",
      participatingAgents: [],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.deepEqual(r.targets, []);
    assert.equal(r.multiRoutedCount, 0);
  });

  it("pure human: undefined roster → empty targets", () => {
    const r = resolveDispatchTargets({
      messageBody: "hello",
      participatingAgents: undefined,
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.deepEqual(r.targets, []);
  });

  it("sole agent bare text auto-routes to that member", () => {
    const r = resolveDispatchTargets({
      messageBody: "fix the flaky test",
      participatingAgents: ["codex"],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.equal(r.targets.length, 1);
    assert.equal(r.targets[0]?.agent, "codex");
    assert.equal(r.targets[0]?.prompt, "fix the flaky test");
    assert.equal(r.multiRoutedCount, 0);
  });

  it("sole agent + unmatched @codex does not wrong-bot activate", () => {
    assert.throws(
      () =>
        resolveDispatchTargets({
          messageBody: "@codex please help",
          participatingAgents: ["claude"],
          installedAgents: installed,
          mentionProfiles: [],
        }),
      /not a member/,
    );
  });

  it("sole agent requires group + 1 human (Hub parity)", () => {
    const r = resolveDispatchTargets({
      messageBody: "hello",
      participatingAgents: ["codex"],
      installedAgents: installed,
      mentionProfiles: [],
      humanMemberCount: 2,
      conversationKind: "group",
    });
    assert.deepEqual(r.targets, []);
  });

  it("multi agent bare text does not fan-out", () => {
    const r = resolveDispatchTargets({
      messageBody: "status update for everyone",
      participatingAgents: ["codex", "claude"],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.deepEqual(r.targets, []);
    assert.equal(r.multiRoutedCount, 0);
  });

  it("explicit @ routes only mentioned members", () => {
    const r = resolveDispatchTargets({
      messageBody: "@codex @claude count off",
      participatingAgents: ["codex", "claude", "grok"],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.equal(r.targets.length, 2);
    assert.equal(r.targets[0]?.agent, "codex");
    assert.equal(r.targets[1]?.agent, "claude");
    assert.equal(r.multiRoutedCount, 2);
  });

  it("throws when @ non-member agent", () => {
    assert.throws(
      () =>
        resolveDispatchTargets({
          messageBody: "@gemini hello",
          participatingAgents: ["codex"],
          installedAgents: installed,
          mentionProfiles: [],
        }),
      /not a member/,
    );
  });

  it("throws when @ agent in pure-human room", () => {
    assert.throws(
      () =>
        resolveDispatchTargets({
          messageBody: "@codex hello",
          participatingAgents: [],
          installedAgents: installed,
          mentionProfiles: [],
        }),
      /not a member/,
    );
  });

  it("sole agent not installed throws clear error", () => {
    assert.throws(
      () =>
        resolveDispatchTargets({
          messageBody: "go",
          participatingAgents: ["opencode"],
          installedAgents: installed,
          mentionProfiles: [],
        }),
      /not installed/,
    );
  });

  it("profile @ routes when member runtime matches", () => {
    const r = resolveDispatchTargets({
      messageBody: "@ResearchGrok dig in",
      participatingAgents: ["grok"],
      installedAgents: installed,
      mentionProfiles: profiles,
    });
    assert.equal(r.targets.length, 1);
    assert.equal(r.targets[0]?.agent, "grok");
    assert.equal(r.targets[0]?.profileId, "profile-research");
    assert.equal(r.multiRoutedCount, 1);
  });
});
