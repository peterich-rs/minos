import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  bumpHostCredentialGeneration,
  enqueueHostCredentialOp,
  getHostCredentialGeneration,
  isHostCredentialCurrent,
  resetHostCredentialControllerForTests,
  revokeHostCredential,
} from "./host-credential-controller.ts";
import { registerHostCredential } from "./ensure-host-connection.ts";

describe("host-credential-controller", () => {
  beforeEach(() => {
    resetHostCredentialControllerForTests();
  });

  it("serializes ops and aborts stale apply after revoke", async () => {
    const applied: string[] = [];
    let releaseA!: () => void;
    const aGate = new Promise<void>((r) => {
      releaseA = r;
    });

    const a = enqueueHostCredentialOp(async (gen) => {
      await aGate;
      if (!isHostCredentialCurrent(gen)) {
        return "aborted-a";
      }
      applied.push("hit_a");
      return "applied-a";
    });

    revokeHostCredential(async () => {
      applied.push("clear");
    });

    const b = enqueueHostCredentialOp(async (gen) => {
      if (!isHostCredentialCurrent(gen)) {
        return "aborted-b";
      }
      applied.push("hit_b");
      return "applied-b";
    });

    releaseA();
    assert.equal(await a, "aborted-a");
    assert.equal(await b, "applied-b");
    assert.deepEqual(applied, ["clear", "hit_b"]);
  });

  it("deferred registerHost never applies token after leave", async () => {
    let releaseRegister!: () => void;
    const registerGate = new Promise<void>((r) => {
      releaseRegister = r;
    });
    const applied: string[] = [];
    const gen = getHostCredentialGeneration();

    const registerPromise = enqueueHostCredentialOp(async (opGen) => {
      return registerHostCredential(
        {
          prepareLink: async () => ({
            deviceId: "host-a",
            publicKey: "pk",
            nonce: "n",
          }),
          signLinkProof: async () => ({ signature: "sig" }),
          registerHost: async () => {
            await registerGate;
            return {
              hostDeviceId: "host-a",
              hostInstallationToken: "hit_account_a",
              pairId: "pair-a",
              accountId: "acc-a",
              hostDisplayName: "Mac A",
              linkedAtMs: 1,
            };
          },
          applyLinkToken: async (token) => {
            applied.push(token);
            return { linked: true };
          },
        },
        "Mac",
        {
          isCurrent: () =>
            isHostCredentialCurrent(opGen) && isHostCredentialCurrent(gen),
        },
      );
    });

    // Switch accounts while A is mid-registerHost.
    revokeHostCredential(async () => {
      applied.push("cleared");
    });

    const bOutcome = await enqueueHostCredentialOp(async (opGen) => {
      return registerHostCredential(
        {
          prepareLink: async () => ({
            deviceId: "host-b",
            publicKey: "pk",
            nonce: "n",
          }),
          signLinkProof: async () => ({ signature: "sig" }),
          registerHost: async () => ({
            hostDeviceId: "host-b",
            hostInstallationToken: "hit_account_b",
            pairId: "pair-b",
            accountId: "acc-b",
            hostDisplayName: "Mac B",
            linkedAtMs: 2,
          }),
          applyLinkToken: async (token) => {
            if (!isHostCredentialCurrent(opGen)) {
              throw new Error("stale apply");
            }
            applied.push(token);
            return { linked: true };
          },
        },
        "Mac",
        { isCurrent: () => isHostCredentialCurrent(opGen) },
      );
    });

    releaseRegister();
    const aOutcome = await registerPromise;

    assert.equal(aOutcome.ok, false);
    assert.equal(bOutcome.ok, true);
    assert.ok(!applied.includes("hit_account_a"));
    assert.ok(applied.includes("hit_account_b"));
    assert.ok(applied.includes("cleared"));
  });

  it("bump invalidates prior generation", () => {
    const g0 = getHostCredentialGeneration();
    assert.equal(isHostCredentialCurrent(g0), true);
    bumpHostCredentialGeneration();
    assert.equal(isHostCredentialCurrent(g0), false);
  });
});
