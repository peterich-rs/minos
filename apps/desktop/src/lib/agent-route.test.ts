import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildAgentMentionOptions,
  parseAgentRouting,
  shortThreadId,
} from "./agent-route.ts";

describe("shortThreadId", () => {
  it("returns up to 8-char prefix", () => {
    assert.equal(shortThreadId("abcdef0123456789"), "abcdef01");
    assert.equal(shortThreadId("abc"), "abc");
  });
});

describe("parseAgentRouting", () => {
  it("parses bare agent and agent#short", () => {
    const bare = parseAgentRouting("@grok hello world");
    assert.equal(bare?.target.agent, "grok");
    assert.equal(bare?.target.threadShortId, undefined);
    assert.equal(bare?.prompt, "hello world");

    const cont = parseAgentRouting("@grok#689035af keep going");
    assert.equal(cont?.target.agent, "grok");
    assert.equal(cont?.target.threadShortId, "689035af");
    assert.equal(cont?.prompt, "keep going");
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
});
