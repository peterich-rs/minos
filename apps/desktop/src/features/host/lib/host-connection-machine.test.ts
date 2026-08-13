import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  decideHostEnsure,
  runHostEnsure,
  type HostEnsureEffectPorts,
} from "./host-connection-machine.ts";

describe("decideHostEnsure", () => {
  it("matching online → no-op", () => {
    assert.deepEqual(
      decideHostEnsure({
        forceReregister: false,
        sessionAccountId: "acc-1",
        hostCredentialAccountId: "acc-1",
        cloudOnline: true,
        hasHostToken: true,
      }),
      { action: "noop-online" },
    );
  });

  it("mismatch account + hasHostToken → force re-register", () => {
    assert.deepEqual(
      decideHostEnsure({
        forceReregister: false,
        sessionAccountId: "acc-2",
        hostCredentialAccountId: "acc-1",
        cloudOnline: true,
        hasHostToken: true,
      }),
      { action: "register" },
    );
  });

  it("matching hasHostToken offline → wait dial", () => {
    assert.deepEqual(
      decideHostEnsure({
        forceReregister: false,
        sessionAccountId: "acc-1",
        hostCredentialAccountId: "acc-1",
        cloudOnline: false,
        hasHostToken: true,
      }),
      { action: "wait-dial" },
    );
  });

  it("forceReregister always registers", () => {
    assert.deepEqual(
      decideHostEnsure({
        forceReregister: true,
        sessionAccountId: "acc-1",
        hostCredentialAccountId: "acc-1",
        cloudOnline: true,
        hasHostToken: true,
      }),
      { action: "register" },
    );
  });

  it("no ownership and no token → register", () => {
    assert.deepEqual(
      decideHostEnsure({
        forceReregister: false,
        sessionAccountId: "acc-1",
        hostCredentialAccountId: null,
        cloudOnline: false,
        hasHostToken: false,
      }),
      { action: "register" },
    );
  });
});

describe("runHostEnsure", () => {
  function ports(
    overrides: Partial<HostEnsureEffectPorts> = {},
  ): HostEnsureEffectPorts {
    return {
      refreshFlags: async () => ({ cloudOnline: false, hasHostToken: false }),
      isOnline: async () => false,
      registerPorts: {
        prepareLink: async () => ({
          deviceId: "host-1",
          publicKey: "pk",
          nonce: "n",
        }),
        signLinkProof: async () => ({ signature: "sig" }),
        applyLinkToken: async () => ({ linked: true }),
        registerHost: async () => ({
          hostDeviceId: "host-1",
          hostInstallationToken: "hit_x",
          pairId: "pair-1",
          accountId: "acc-1",
          hostDisplayName: "This Mac",
          linkedAtMs: 1,
        }),
      },
      hostDisplayName: "This Mac",
      waitOpts: { timeoutMs: 10, intervalMs: 1 },
      ...overrides,
    };
  }

  it("returns online without register when matching online", async () => {
    let registered = false;
    const result = await runHostEnsure(
      {
        forceReregister: false,
        sessionAccountId: "acc-1",
        hostCredentialAccountId: "acc-1",
      },
      ports({
        refreshFlags: async () => ({ cloudOnline: true, hasHostToken: true }),
        registerPorts: {
          prepareLink: async () => {
            registered = true;
            throw new Error("should not prepare");
          },
          signLinkProof: async () => ({ signature: "x" }),
          applyLinkToken: async () => ({ linked: true }),
          registerHost: async () => {
            throw new Error("should not register");
          },
        },
      }),
    );
    assert.equal(result.kind, "online");
    assert.equal(registered, false);
  });

  it("registers when account ownership mismatches despite token", async () => {
    const result = await runHostEnsure(
      {
        forceReregister: false,
        sessionAccountId: "acc-2",
        hostCredentialAccountId: "acc-1",
      },
      ports({
        refreshFlags: async () => ({ cloudOnline: true, hasHostToken: true }),
        isOnline: async () => true,
      }),
    );
    assert.equal(result.kind, "registered-online");
  });
});
