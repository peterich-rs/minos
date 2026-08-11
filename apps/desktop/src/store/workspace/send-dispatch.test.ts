import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildStructuredMentions,
  resolveDispatchTargets,
} from "./resolve-dispatch-targets.ts";
import type {
  MentionHuman,
  MentionProfile,
} from "../../shared/lib/agent-route.ts";

const installed = new Set(["codex", "claude", "grok"]);
const profiles: MentionProfile[] = [
  { id: "profile-research", name: "ResearchGrok", runtimeAgent: "grok" },
];
const runtimeProfiles: MentionProfile[] = [
  { id: "bot-codex", name: "Codex", runtimeAgent: "codex" },
  { id: "bot-claude", name: "Claude", runtimeAgent: "claude" },
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

  it("throws when named profile runtime is not on roster", () => {
    assert.throws(
      () =>
        resolveDispatchTargets({
          messageBody: "@ResearchGrok dig in",
          participatingAgents: ["codex"],
          installedAgents: installed,
          mentionProfiles: profiles,
        }),
      /not a member/,
    );
  });

  it("does not route unjoined profile not present in roster-scoped mentionProfiles", () => {
    // Multi-member room + bare text → pure human (no fan-out). Profile absent
    // from mentionProfiles cannot be @-resolved even if Host still has it.
    const r = resolveDispatchTargets({
      messageBody: "hello team",
      participatingAgents: ["codex", "claude"],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.deepEqual(r.targets, []);
  });

  it("unknown @Name with empty roster profiles does not profile-route (sole may still apply)", () => {
    // ResearchGrok is not a known runtime and not in mentionProfiles → no profile
    // target. Sole-bot room may still auto-route bare intent (Hub parity for
    // non-agentish tokens). Callers must not pass unjoined profiles in mentionProfiles.
    const r = resolveDispatchTargets({
      messageBody: "@ResearchGrok dig in",
      participatingAgents: ["grok"],
      installedAgents: installed,
      mentionProfiles: [],
    });
    assert.equal(r.targets.length, 1);
    assert.equal(r.targets[0]?.agent, "grok");
    assert.equal(r.targets[0]?.profileId, undefined);
  });
});

describe("buildStructuredMentions", () => {
  it("maps profile @Name to bot_id with start/length", () => {
    const body = "@ResearchGrok dig in";
    const mentions = buildStructuredMentions(body, profiles);
    assert.equal(mentions.length, 1);
    assert.deepEqual(mentions[0], {
      kind: "bot",
      bot_id: "profile-research",
      start: 0,
      length: "@ResearchGrok".length,
    });
  });

  it("maps bare runtime to roster profile bot_id", () => {
    const body = "@codex fix the flaky test";
    const mentions = buildStructuredMentions(body, runtimeProfiles);
    assert.equal(mentions.length, 1);
    assert.equal(mentions[0]?.kind, "bot");
    if (mentions[0]?.kind === "bot") {
      assert.equal(mentions[0].bot_id, "bot-codex");
      assert.equal(mentions[0].start, 0);
      assert.equal(mentions[0].length, "@codex".length);
    }
  });

  it("maps @p/<id> profile form", () => {
    const body = "please @p/profile-research review";
    const mentions = buildStructuredMentions(body, profiles);
    assert.equal(mentions.length, 1);
    assert.deepEqual(mentions[0], {
      kind: "bot",
      bot_id: "profile-research",
      start: "please ".length,
      length: "@p/profile-research".length,
    });
  });

  it("dedupes repeated bot mentions and preserves appearance order", () => {
    const body = "@codex then @claude and @codex again";
    const mentions = buildStructuredMentions(body, runtimeProfiles);
    assert.equal(mentions.length, 2);
    assert.equal(mentions[0]?.kind, "bot");
    assert.equal(mentions[1]?.kind, "bot");
    if (mentions[0]?.kind === "bot" && mentions[1]?.kind === "bot") {
      assert.equal(mentions[0].bot_id, "bot-codex");
      assert.equal(mentions[1].bot_id, "bot-claude");
    }
  });

  it("returns empty when bare runtime has no roster profile mapping", () => {
    const mentions = buildStructuredMentions("@codex hello", []);
    assert.deepEqual(mentions, []);
  });

  it("includes human @minos_id when mentionHumans provided", () => {
    const humans: MentionHuman[] = [
      {
        accountId: "acct-alice",
        minosId: "alice",
        displayName: "Alice",
      },
    ];
    const body = "hey @alice look at this";
    const mentions = buildStructuredMentions(body, runtimeProfiles, {
      mentionHumans: humans,
    });
    assert.equal(mentions.length, 1);
    assert.deepEqual(mentions[0], {
      kind: "account",
      account_id: "acct-alice",
      start: "hey ".length,
      length: "@alice".length,
    });
  });

  it("mixes bot and human mentions", () => {
    const humans: MentionHuman[] = [
      {
        accountId: "acct-bob",
        minosId: "bob",
        displayName: "Bob",
      },
    ];
    const body = "@bob @codex ship it";
    const mentions = buildStructuredMentions(body, runtimeProfiles, {
      mentionHumans: humans,
    });
    assert.equal(mentions.length, 2);
    assert.equal(mentions[0]?.kind, "account");
    assert.equal(mentions[1]?.kind, "bot");
  });
});
