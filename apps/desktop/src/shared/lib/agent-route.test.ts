import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildAgentMentionOptions,
  isProfileNameCleanToken,
  isProfileNameUnique,
  parseAgentRouting,
  profileMentionInsert,
  shortSessionId,
  validateProfileName,
  type MentionProfile,
} from "./agent-route.ts";

const profiles: MentionProfile[] = [
  { id: "profile-research", name: "ResearchGrok", runtimeAgent: "grok" },
  { id: "profile-dup-a", name: "Helper", runtimeAgent: "codex" },
  { id: "profile-dup-b", name: "Helper", runtimeAgent: "claude" },
  { id: "profile-named-codex", name: "codex", runtimeAgent: "claude" },
  { id: "profile-spacey", name: "My Helper", runtimeAgent: "grok" },
  { id: "profile-hash", name: "tag#one", runtimeAgent: "codex" },
];

describe("shortSessionId", () => {
  it("returns up to 8-char prefix", () => {
    assert.equal(shortSessionId("abcdef0123456789"), "abcdef01");
    assert.equal(shortSessionId("abc"), "abc");
  });
});

describe("parseAgentRouting", () => {
  it("parses bare agent and agent#short", () => {
    const bare = parseAgentRouting("@grok hello world");
    assert.equal(bare?.target.agent, "grok");
    assert.equal(bare?.target.sessionShortId, undefined);
    assert.equal(bare?.target.profileId, undefined);
    assert.equal(bare?.prompt, "hello world");

    const cont = parseAgentRouting("@grok#689035af keep going");
    assert.equal(cont?.target.agent, "grok");
    assert.equal(cont?.target.sessionShortId, "689035af");
    assert.equal(cont?.prompt, "keep going");
  });

  it("parses unique profile name to profileId + runtime agent", () => {
    const routed = parseAgentRouting("@ResearchGrok dig in", profiles);
    assert.equal(routed?.target.agent, "grok");
    assert.equal(routed?.target.profileId, "profile-research");
    assert.equal(routed?.prompt, "dig in");
  });

  it("matches profile names case-insensitively", () => {
    const routed = parseAgentRouting("@researchgrok x", profiles);
    assert.equal(routed?.target.profileId, "profile-research");
  });

  it("parses @p/<id> profile tokens", () => {
    const routed = parseAgentRouting("@p/profile-research go", profiles);
    assert.equal(routed?.target.profileId, "profile-research");
    assert.equal(routed?.target.agent, "grok");
  });

  it("prefers runtime agent over profile when names collide", () => {
    const routed = parseAgentRouting("@codex hi", profiles);
    assert.equal(routed?.target.agent, "codex");
    assert.equal(routed?.target.profileId, undefined);
  });

  it("does not resolve ambiguous profile names without @p/id", () => {
    assert.equal(parseAgentRouting("@Helper x", profiles), null);
  });
});

describe("profile name validation", () => {
  it("accepts clean tokens and rejects whitespace/#/@", () => {
    assert.equal(isProfileNameCleanToken("ResearchGrok"), true);
    assert.equal(isProfileNameCleanToken("my-agent_1"), true);
    assert.equal(isProfileNameCleanToken("My Helper"), false);
    assert.equal(isProfileNameCleanToken("tag#one"), false);
    assert.equal(isProfileNameCleanToken("at@x"), false);
    assert.equal(isProfileNameCleanToken("  "), false);

    assert.equal(validateProfileName("ResearchGrok"), null);
    assert.equal(validateProfileName(""), "Name is required");
    assert.equal(
      validateProfileName("My Helper"),
      "Name cannot contain spaces, #, or @",
    );
    assert.equal(
      validateProfileName("tag#one"),
      "Name cannot contain spaces, #, or @",
    );
    assert.equal(
      validateProfileName("at@x"),
      "Name cannot contain spaces, #, or @",
    );
  });
});

describe("profile mention uniqueness", () => {
  it("flags runtime and duplicate name collisions", () => {
    assert.equal(isProfileNameUnique("ResearchGrok", profiles), true);
    assert.equal(isProfileNameUnique("Helper", profiles), false);
    assert.equal(isProfileNameUnique("codex", profiles), false);
  });

  it("inserts @Name when unique clean token else @p/id", () => {
    assert.equal(
      profileMentionInsert(profiles[0]!, profiles),
      "@ResearchGrok ",
    );
    assert.equal(
      profileMentionInsert(profiles[1]!, profiles),
      "@p/profile-dup-a ",
    );
    assert.equal(
      profileMentionInsert(profiles[3]!, profiles),
      "@p/profile-named-codex ",
    );
    // Unique but not a clean @-token → force id form.
    assert.equal(
      profileMentionInsert(profiles[4]!, profiles),
      "@p/profile-spacey ",
    );
    assert.equal(
      profileMentionInsert(profiles[5]!, profiles),
      "@p/profile-hash ",
    );
  });
});

describe("buildAgentMentionOptions", () => {
  it("always offers bare @agent even when sessions exist for that agent", () => {
    const opts = buildAgentMentionOptions(
      "grok",
      [
        { agent: "grok", installed: true, status: "ok" },
        { agent: "opencode", installed: true, status: "ok" },
      ],
      [
        {
          id: "689035af-398",
          agent: "grok",
          shortId: "689035af",
          status: "idle",
        },
        {
          id: "a27a67a7-a39",
          agent: "grok",
          shortId: "a27a67a7",
          status: "running",
        },
      ],
    );
    const labels = opts.map((o) => o.label);
    assert.ok(labels.includes("@grok"), `expected bare @grok in ${labels}`);
    assert.ok(labels.includes("@grok#689035af"));
    assert.ok(labels.includes("@grok#a27a67a7"));
    assert.equal(opts.find((o) => o.label === "@grok")?.hint, "new session");
    assert.match(
      opts.find((o) => o.label === "@grok#689035af")?.hint ?? "",
      /continue/,
    );
  });

  it("includes profile options with runtime hint", () => {
    const opts = buildAgentMentionOptions(
      "research",
      [{ agent: "grok", installed: true, status: "ok" }],
      [],
      profiles,
    );
    const profileOpt = opts.find((o) => o.id === "profile:profile-research");
    assert.ok(profileOpt, `expected profile option in ${opts.map((o) => o.id)}`);
    assert.equal(profileOpt?.label, "@ResearchGrok");
    assert.match(profileOpt?.hint ?? "", /profile · grok/);
    assert.equal(profileOpt?.insert, "@ResearchGrok ");
  });

  it("hides done/failed sessions from continue list", () => {
    const opts = buildAgentMentionOptions(
      "",
      [{ agent: "codex", installed: true, status: "ok" }],
      [
        {
          id: "done-1",
          agent: "codex",
          shortId: "deadbeef",
          status: "done",
        },
        {
          id: "live-1",
          agent: "codex",
          shortId: "cafebabe",
          status: "idle",
        },
      ],
    );
    const labels = opts.map((o) => o.label);
    assert.ok(labels.includes("@codex"));
    assert.ok(labels.includes("@codex#cafebabe"));
    assert.ok(!labels.includes("@codex#deadbeef"));
  });

  it("disables uninstalled bare agents", () => {
    const opts = buildAgentMentionOptions(
      "open",
      [{ agent: "opencode", installed: false, status: "missing" }],
      [],
    );
    assert.equal(opts[0]?.label, "@opencode");
    assert.equal(opts[0]?.disabled, true);
  });

  it("gates options to conversation member roster", () => {
    const opts = buildAgentMentionOptions({
      query: "",
      clis: [
        { agent: "codex", installed: true, status: "ok" },
        { agent: "claude", installed: true, status: "ok" },
        { agent: "grok", installed: true, status: "ok" },
      ],
      sessions: [
        {
          id: "s-claude",
          agent: "claude",
          shortId: "aabbccdd",
          status: "idle",
        },
        {
          id: "s-grok",
          agent: "grok",
          shortId: "eeff0011",
          status: "idle",
        },
      ],
      profiles: [
        {
          id: "profile-research",
          name: "ResearchGrok",
          runtimeAgent: "grok",
        },
        {
          id: "profile-code",
          name: "CodeClaude",
          runtimeAgent: "claude",
        },
      ],
      memberAgents: ["claude"],
    });
    const labels = opts.map((o) => o.label);
    assert.ok(labels.includes("@claude"));
    assert.ok(labels.includes("@claude#aabbccdd"));
    assert.ok(labels.includes("@CodeClaude"));
    assert.ok(!labels.includes("@codex"));
    assert.ok(!labels.includes("@grok"));
    assert.ok(!labels.includes("@grok#eeff0011"));
    assert.ok(!labels.includes("@ResearchGrok"));
  });

  it("returns no options when roster is empty", () => {
    const opts = buildAgentMentionOptions({
      query: "",
      clis: [{ agent: "codex", installed: true, status: "ok" }],
      sessions: [],
      memberAgents: [],
    });
    assert.deepEqual(opts, []);
  });
});
