import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  displayNameForRuntime,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "./im-cloud-sync-helpers.ts";

describe("normalizeHostRuntime", () => {
  it("lowercases known agent bin names", () => {
    assert.equal(normalizeHostRuntime("Codex"), "codex");
    assert.equal(normalizeHostRuntime(" claude "), "claude");
    assert.equal(normalizeHostRuntime("gemini"), "gemini");
    assert.equal(normalizeHostRuntime("opencode"), "opencode");
    assert.equal(normalizeHostRuntime("grok"), "grok");
  });

  it("rejects unknown names so callers never treat bin names as cloud agent_ids", () => {
    assert.equal(normalizeHostRuntime("bot-uuid-here"), null);
    assert.equal(normalizeHostRuntime("agent_codex"), null);
    assert.equal(normalizeHostRuntime(""), null);
    assert.equal(normalizeHostRuntime(null), null);
    assert.equal(normalizeHostRuntime(undefined), null);
  });
});

describe("displayNameForRuntime", () => {
  it("capitalizes the first letter", () => {
    assert.equal(displayNameForRuntime("codex"), "Codex");
    assert.equal(displayNameForRuntime("claude"), "Claude");
  });
});

describe("isProjectableAgentMessage", () => {
  it("accepts agent role and agent-result ids with body", () => {
    assert.equal(
      isProjectableAgentMessage({
        id: "agent-result:c:s:t",
        role: "agent",
        body: "done",
      }),
      true,
    );
    assert.equal(
      isProjectableAgentMessage({
        id: "agent-result:c:s:t",
        role: "system",
        body: "done",
      }),
      true,
    );
    assert.equal(
      isProjectableAgentMessage({
        id: "msg-1",
        role: "agent",
        body: "hi",
      }),
      true,
    );
  });

  it("rejects user rows and empty bodies", () => {
    assert.equal(
      isProjectableAgentMessage({
        id: "u1",
        role: "user",
        body: "hello",
      }),
      false,
    );
    assert.equal(
      isProjectableAgentMessage({
        id: "agent-result:x",
        role: "agent",
        body: "   ",
      }),
      false,
    );
    assert.equal(
      isProjectableAgentMessage({
        id: "",
        role: "agent",
        body: "x",
      }),
      false,
    );
  });
});
