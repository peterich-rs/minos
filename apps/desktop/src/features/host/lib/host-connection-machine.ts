/**
 * Pure host-connection ensure decision + effect orchestration.
 *
 * account-store owns session + generation counters; this module decides whether
 * to no-op, wait for dial, or force re-register, then runs the effect ports.
 */

import {
  registerHostCredential,
  waitForCloudOnline,
  type EnsureHostPorts,
  type EnsureHostOutcome,
} from "./ensure-host-connection.ts";

export type HostEnsureInput = {
  forceReregister: boolean;
  sessionAccountId: string;
  hostCredentialAccountId: string | null;
  cloudOnline: boolean;
  hasHostToken: boolean;
};

export type HostEnsureDecision =
  | { action: "noop-online" }
  | { action: "wait-dial" }
  | { action: "register" };

/**
 * Decide host ensure path.
 * - mismatch account + hasHostToken (or no ownership) → register
 * - matching ownership + online → no-op
 * - matching ownership + hasHostToken → wait dial
 * - otherwise → register
 */
export function decideHostEnsure(input: HostEnsureInput): HostEnsureDecision {
  const accountId = input.sessionAccountId.trim();
  if (!accountId) return { action: "register" };

  const hostOwnedBySession = input.hostCredentialAccountId === accountId;

  if (input.forceReregister) {
    return { action: "register" };
  }

  // Foreign or missing ownership with a lingering hit_ must re-register.
  if (!hostOwnedBySession) {
    return { action: "register" };
  }

  if (input.cloudOnline) {
    return { action: "noop-online" };
  }

  if (input.hasHostToken) {
    return { action: "wait-dial" };
  }

  return { action: "register" };
}

export type HostEnsureEffectPorts = {
  refreshFlags: () => Promise<{ cloudOnline: boolean; hasHostToken: boolean }>;
  isOnline: () => Promise<boolean>;
  registerPorts: EnsureHostPorts;
  hostDisplayName: string;
  waitOpts?: { timeoutMs?: number; intervalMs?: number };
};

export type HostEnsureRunResult =
  | { kind: "online" }
  | { kind: "offline"; message: string; hostCredentialAccountId?: null }
  | {
      kind: "registered-online";
      hostInstallationId: string;
      hostDisplayName: string;
      linkedAtMs: number;
      pairId: string;
    }
  | {
      kind: "registered-offline";
      hostInstallationId: string;
      hostDisplayName: string;
      linkedAtMs: number;
      pairId: string;
      message: string;
    }
  | {
      kind: "register-failed";
      stage: string;
      message: string;
    };

/**
 * Run ensure decision against live flags + register/wait ports.
 * Caller supplies current ownership snapshot; generation checks stay outside.
 */
export async function runHostEnsure(
  decisionInput: Omit<HostEnsureInput, "cloudOnline" | "hasHostToken">,
  ports: HostEnsureEffectPorts,
): Promise<HostEnsureRunResult> {
  const flags = await ports.refreshFlags();
  const decision = decideHostEnsure({
    ...decisionInput,
    cloudOnline: flags.cloudOnline,
    hasHostToken: flags.hasHostToken,
  });

  if (decision.action === "noop-online") {
    return { kind: "online" };
  }

  if (decision.action === "wait-dial") {
    const online = await waitForCloudOnline(ports.isOnline, ports.waitOpts);
    if (online) return { kind: "online" };
    return {
      kind: "offline",
      message:
        "Has local host credential but server is unreachable. Check minos-backend, then Retry.",
    };
  }

  const outcome: EnsureHostOutcome = await registerHostCredential(
    ports.registerPorts,
    ports.hostDisplayName,
  );
  if (!outcome.ok) {
    return {
      kind: "register-failed",
      stage: outcome.stage,
      message: outcome.message,
    };
  }

  const online = await waitForCloudOnline(ports.isOnline, ports.waitOpts);
  if (online) {
    return {
      kind: "registered-online",
      hostInstallationId: outcome.hostInstallationId,
      hostDisplayName: outcome.hostDisplayName,
      linkedAtMs: outcome.linkedAtMs,
      pairId: outcome.pairId,
    };
  }
  return {
    kind: "registered-offline",
    hostInstallationId: outcome.hostInstallationId,
    hostDisplayName: outcome.hostDisplayName,
    linkedAtMs: outcome.linkedAtMs,
    pairId: outcome.pairId,
    message:
      "Signed in but not connected to the server yet. Retry or check that minos-backend is running.",
  };
}
