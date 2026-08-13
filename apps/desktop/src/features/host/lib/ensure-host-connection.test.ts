import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  registerHostCredential,
  waitForCloudOnline,
  type EnsureHostPorts,
} from "./ensure-host-connection.ts";

function ports(overrides: Partial<EnsureHostPorts> = {}): EnsureHostPorts {
  return {
    prepareLink: async () => ({
      deviceId: "host-1",
      publicKey: "ed25519:pk",
      nonce: "nonce_abc",
    }),
    signLinkProof: async () => ({ signature: "ed25519-sig:sig" }),
    applyLinkToken: async () => ({ linked: true }),
    registerHost: async () => ({
      hostDeviceId: "host-1",
      hostInstallationToken: "hit_token",
      pairId: "pair-1",
      accountId: "acc-1",
      hostDisplayName: "Studio Mac",
      linkedAtMs: 42,
    }),
    ...overrides,
  };
}

describe("registerHostCredential", () => {
  it("runs prepare → sign → cloud → apply once", async () => {
    const calls: string[] = [];
    const outcome = await registerHostCredential(
      ports({
        prepareLink: async () => {
          calls.push("prepare");
          return {
            deviceId: "host-1",
            publicKey: "ed25519:pk",
            nonce: "nonce_abc",
          };
        },
        signLinkProof: async (id, nonce) => {
          calls.push(`sign:${id}:${nonce}`);
          return { signature: "ed25519-sig:sig" };
        },
        registerHost: async (input) => {
          calls.push(`cloud:${input.deviceId}`);
          return {
            hostDeviceId: "host-1",
            hostInstallationToken: "hit_token",
            pairId: "pair-1",
            accountId: "acc-1",
            hostDisplayName: input.hostDisplayName,
            linkedAtMs: 99,
          };
        },
        applyLinkToken: async (token) => {
          calls.push(`apply:${token}`);
          return { linked: true };
        },
      }),
      "This Mac",
    );
    assert.equal(outcome.ok, true);
    assert.deepEqual(calls, [
      "prepare",
      "sign:host-1:nonce_abc",
      "cloud:host-1",
      "apply:hit_token",
    ]);
  });

  it("stops at cloud failure without apply", async () => {
    let applied = false;
    const outcome = await registerHostCredential(
      ports({
        registerHost: async () => {
          throw new Error("conflict");
        },
        applyLinkToken: async () => {
          applied = true;
          return { linked: true };
        },
      }),
      "Mac",
    );
    assert.equal(outcome.ok, false);
    assert.equal(applied, false);
  });
});

describe("waitForCloudOnline", () => {
  it("returns true when isOnline becomes true", async () => {
    let n = 0;
    const ok = await waitForCloudOnline(
      async () => {
        n += 1;
        return n >= 2;
      },
      { timeoutMs: 2_000, intervalMs: 10 },
    );
    assert.equal(ok, true);
  });
});
