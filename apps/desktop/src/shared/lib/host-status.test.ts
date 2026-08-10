import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  PROJECT_HOST_THIS_MAC,
  cloudModeFromAccountSync,
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

  it("shows Online when Account sync is online (primary)", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      accountSync: "online",
      hubOnline: false,
    });
    assert.equal(p.label, "Online");
    assert.equal(p.cloud, "online");
    assert.equal(p.tone, "ready");
    assert.equal(p.hostReady, false);
    assert.equal(p.hostLabel, "Host offline");
  });

  it("does not show Online when only Host is live and Account is offline", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      accountSync: "offline",
      hubOnline: true,
    });
    assert.equal(p.label, "Offline");
    assert.equal(p.cloud, "offline");
    assert.equal(p.hostReady, true);
    assert.equal(p.hostLabel, "Host ready");
  });

  it("shows Connecting… while Account is connecting", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      accountSync: "connecting",
    });
    assert.equal(p.label, "Connecting…");
    assert.equal(p.tone, "connecting");
  });

  it("shows Offline when Account is offline (local runtime still ready)", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      accountSync: "offline",
    });
    assert.equal(p.label, "Offline");
    assert.equal(p.runtimeReady, true);
  });

  it("legacy: maps hubOnline true to Online when accountSync/cloud omitted", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      hubOnline: true,
    });
    assert.equal(p.label, "Online");
  });

  it("Account online + Host offline: humans can chat, bots not ready", () => {
    const p = deriveHostPresence({
      source: "daemon",
      daemonConnected: true,
      accountSync: "online",
      hubOnline: false,
    });
    assert.equal(p.label, "Online");
    assert.equal(p.cloud, "online");
    assert.equal(p.hostReady, false);
    assert.equal(p.hostLabel, "Host offline");
    assert.equal(p.runtimeReady, true);
  });
});

describe("cloudModeFromAccountSync", () => {
  it("maps live/syncing to online", () => {
    assert.equal(cloudModeFromAccountSync("live"), "online");
    assert.equal(cloudModeFromAccountSync("syncing"), "online");
  });
  it("maps connecting and offline states", () => {
    assert.equal(cloudModeFromAccountSync("connecting"), "connecting");
    assert.equal(cloudModeFromAccountSync("disconnected"), "offline");
    assert.equal(cloudModeFromAccountSync("error"), "offline");
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
