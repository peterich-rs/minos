import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  runHostLinkFlow,
  runHostUnlinkFlow,
  type HostLinkPorts,
} from "./host-link-flow.ts";

function ports(overrides: Partial<HostLinkPorts> = {}): HostLinkPorts {
  return {
    prepareLink: async () => ({
      installationId: "host-1",
      publicKey: "ed25519:pk",
      nonce: "nonce_abc",
    }),
    signLinkProof: async () => ({ signature: "ed25519-sig:sig" }),
    applyLinkToken: async () => ({ linked: true }),
    linkHost: async () => ({
      hostInstallationId: "host-1",
      hostInstallationToken: "hit_token",
      pairId: "pair-1",
      accountId: "acc-1",
      hostDisplayName: "Studio Mac",
      linkedAtMs: 42,
    }),
    ...overrides,
  };
}

describe("runHostLinkFlow", () => {
  it("completes prepare → sign → cloud → apply", async () => {
    const calls: string[] = [];
    const outcome = await runHostLinkFlow(
      ports({
        prepareLink: async () => {
          calls.push("prepare");
          return {
            installationId: "host-1",
            publicKey: "ed25519:pk",
            nonce: "nonce_abc",
          };
        },
        signLinkProof: async (id, nonce) => {
          calls.push(`sign:${id}:${nonce}`);
          return { signature: "ed25519-sig:sig" };
        },
        linkHost: async (input) => {
          calls.push(
            `cloud:${input.installationId}:${input.signature}:${input.hostDisplayName}`,
          );
          return {
            hostInstallationId: "host-1",
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
    assert.equal(outcome.linked, true);
    if (outcome.linked) {
      assert.equal(outcome.hostInstallationId, "host-1");
      assert.equal(outcome.hostDisplayName, "This Mac");
      assert.equal(outcome.linkedAtMs, 99);
    }
    assert.deepEqual(calls, [
      "prepare",
      "sign:host-1:nonce_abc",
      "cloud:host-1:ed25519-sig:sig:This Mac",
      "apply:hit_token",
    ]);
  });

  it("stops at prepare failure", async () => {
    const outcome = await runHostLinkFlow(
      ports({
        prepareLink: async () => {
          throw new Error("relay offline");
        },
      }),
      "Mac",
    );
    assert.equal(outcome.linked, false);
    if (!outcome.linked) {
      assert.equal(outcome.stage, "prepare");
      assert.match(outcome.message, /relay offline/);
    }
  });

  it("stops at cloud failure without apply", async () => {
    let applied = false;
    const outcome = await runHostLinkFlow(
      ports({
        linkHost: async () => {
          throw new Error("host_linked_elsewhere");
        },
        applyLinkToken: async () => {
          applied = true;
          return { linked: true };
        },
      }),
      "Mac",
    );
    assert.equal(outcome.linked, false);
    if (!outcome.linked) assert.equal(outcome.stage, "cloud");
    assert.equal(applied, false);
  });

  it("fails when apply returns linked=false", async () => {
    const outcome = await runHostLinkFlow(
      ports({
        applyLinkToken: async () => ({ linked: false }),
      }),
      "Mac",
    );
    assert.equal(outcome.linked, false);
    if (!outcome.linked) assert.equal(outcome.stage, "apply");
  });
});

describe("runHostUnlinkFlow", () => {
  it("unlinks by installation id", async () => {
    let seen: string | null = null;
    const outcome = await runHostUnlinkFlow(
      {
        unlinkHost: async (id) => {
          seen = id;
        },
      },
      "host-9",
    );
    assert.equal(outcome.ok, true);
    assert.equal(seen, "host-9");
  });

  it("rejects empty id", async () => {
    const outcome = await runHostUnlinkFlow({ unlinkHost: async () => {} }, "  ");
    assert.equal(outcome.ok, false);
  });
});
