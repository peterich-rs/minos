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

  it("shows Online when cloud is online", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      cloud: "online",
    });
    assert.equal(p.label, "Online");
    assert.equal(p.cloud, "online");
    assert.equal(p.tone, "ready");
  });

  it("shows Connecting… while binding / dialing hub", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      cloud: "connecting",
    });
    assert.equal(p.label, "Connecting…");
    assert.equal(p.tone, "connecting");
  });

  it("shows Offline when cloud is offline (local runtime still ready)", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      cloud: "offline",
    });
    assert.equal(p.label, "Offline");
    assert.equal(p.runtimeReady, true);
  });

  it("maps hubOnline true to Online when cloud omitted", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      hubOnline: true,
    });
    assert.equal(p.label, "Online");
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
