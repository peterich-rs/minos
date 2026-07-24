import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  PROJECT_HOST_THIS_MAC,
  deriveHostPresence,
  projectHostLabel,
} from "./host-status.ts";

describe("deriveHostPresence", () => {
  it("shows Preview for browser mock data", () => {
    const p = deriveHostPresence({
      source: "mock",
      daemonConnected: false,
    });
    assert.equal(p.tone, "preview");
    assert.equal(p.label, "Preview");
    assert.equal(p.runtimeReady, false);
    assert.equal(p.linkLabel, "Local only");
  });

  it("shows Unavailable when daemon is not connected", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: false,
    });
    assert.equal(p.tone, "unavailable");
    assert.equal(p.label, "Unavailable");
    assert.equal(p.runtimeReady, false);
  });

  it("defaults to Ready · Local only when daemon is up (v1, no relay)", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
    });
    assert.equal(p.tone, "ready");
    assert.equal(p.label, "Ready · Local only");
    assert.equal(p.readinessLabel, "Ready");
    assert.equal(p.linkMode, "local_only");
    assert.equal(p.runtimeReady, true);
  });

  it("shows Ready · Linked when relay is linked", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      relayLinked: true,
    });
    assert.equal(p.label, "Ready · Linked");
    assert.equal(p.linkMode, "linked");
    assert.equal(p.linkLabel, "Linked");
  });

  it("treats explicit relayLinked false as Local only", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      relayLinked: false,
    });
    assert.equal(p.label, "Ready · Local only");
  });
});

describe("projectHostLabel", () => {
  it("defaults to This Mac", () => {
    assert.equal(projectHostLabel(), PROJECT_HOST_THIS_MAC);
    assert.equal(projectHostLabel(null), PROJECT_HOST_THIS_MAC);
    assert.equal(projectHostLabel("  "), PROJECT_HOST_THIS_MAC);
  });

  it("uses remote host name when provided", () => {
    assert.equal(projectHostLabel("Office Mac"), "Office Mac");
  });
});
