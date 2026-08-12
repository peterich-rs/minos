/**
 * One-time silent host registration with the cloud.
 *
 * Product: login on this Mac owns host control — users never "Link".
 * Protocol still needs a durable `hit_` for `/ws/host`. This module runs
 * **only when that credential is missing or must be re-issued** — not on
 * every ensure/poll (re-register rotates/revokes hit_ and races the dialer).
 *
 * Flow: prepare → sign → POST /v1/hosts/register-equivalent → apply hit_
 * (HTTP path remains `/v1/hosts/link` until backend renames).
 */

export type EnsureHostPorts = {
  prepareLink: () => Promise<{
    installationId: string;
    publicKey: string;
    nonce: string;
  }>;
  signLinkProof: (
    installationId: string,
    nonce: string,
  ) => Promise<{ signature: string }>;
  applyLinkToken: (token: string) => Promise<{ linked: boolean }>;
  registerHost: (input: {
    installationId: string;
    nonce: string;
    publicKey: string;
    signature: string;
    hostDisplayName: string;
  }) => Promise<{
    hostInstallationId: string;
    hostInstallationToken: string;
    pairId: string;
    accountId: string;
    hostDisplayName: string;
    linkedAtMs: number;
  }>;
};

export type EnsureHostSuccess = {
  ok: true;
  hostInstallationId: string;
  hostDisplayName: string;
  linkedAtMs: number;
  pairId: string;
  accountId: string;
};

export type EnsureHostFailure = {
  ok: false;
  stage: "prepare" | "sign" | "cloud" | "apply";
  message: string;
  cause?: unknown;
};

export type EnsureHostOutcome = EnsureHostSuccess | EnsureHostFailure;

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return String(error);
}

export type RegisterHostCredentialOptions = {
  /**
   * Re-checked after every await and again immediately before applyLinkToken.
   * Account leave/switch must return false so a deferred prior register never
   * writes hit_ under a newer account.
   */
  isCurrent?: () => boolean;
};

function abortedOutcome(): EnsureHostFailure {
  return {
    ok: false,
    stage: "apply",
    message: "Host registration superseded by account leave/switch",
  };
}

/**
 * Register this Mac with the account and apply a fresh `hit_`.
 * Call only when local credential is absent or known-invalid — not every poll.
 *
 * Generation checks: every await boundary + immediately before applyLinkToken.
 */
export async function registerHostCredential(
  ports: EnsureHostPorts,
  hostDisplayName: string,
  opts?: RegisterHostCredentialOptions,
): Promise<EnsureHostOutcome> {
  const displayName = hostDisplayName.trim() || "This Mac";
  const stillCurrent = () => opts?.isCurrent?.() ?? true;

  if (!stillCurrent()) return abortedOutcome();

  let prepared: {
    installationId: string;
    publicKey: string;
    nonce: string;
  };
  try {
    prepared = await ports.prepareLink();
  } catch (error) {
    return {
      ok: false,
      stage: "prepare",
      message: errorMessage(error),
      cause: error,
    };
  }
  if (!stillCurrent()) return abortedOutcome();

  let signature: string;
  try {
    const signed = await ports.signLinkProof(
      prepared.installationId,
      prepared.nonce,
    );
    signature = signed.signature;
  } catch (error) {
    return {
      ok: false,
      stage: "sign",
      message: errorMessage(error),
      cause: error,
    };
  }
  if (!stillCurrent()) return abortedOutcome();

  let cloud: Awaited<ReturnType<EnsureHostPorts["registerHost"]>>;
  try {
    cloud = await ports.registerHost({
      installationId: prepared.installationId,
      nonce: prepared.nonce,
      publicKey: prepared.publicKey,
      signature,
      hostDisplayName: displayName,
    });
  } catch (error) {
    return {
      ok: false,
      stage: "cloud",
      message: errorMessage(error),
      cause: error,
    };
  }
  if (!stillCurrent()) return abortedOutcome();

  try {
    // Final guard immediately before the only write that mutates daemon hit_.
    if (!stillCurrent()) return abortedOutcome();
    const applied = await ports.applyLinkToken(cloud.hostInstallationToken);
    if (!stillCurrent()) {
      // Applied under a race — caller must clear; surface as aborted so setState
      // does not claim ownership for the superseded account.
      return abortedOutcome();
    }
    if (!applied.linked) {
      return {
        ok: false,
        stage: "apply",
        message: "Daemon did not accept host installation token",
      };
    }
  } catch (error) {
    return {
      ok: false,
      stage: "apply",
      message: errorMessage(error),
      cause: error,
    };
  }

  return {
    ok: true,
    hostInstallationId: cloud.hostInstallationId,
    hostDisplayName: cloud.hostDisplayName || displayName,
    linkedAtMs: cloud.linkedAtMs,
    pairId: cloud.pairId,
    accountId: cloud.accountId,
  };
}

/** @deprecated Prefer registerHostCredential — same behavior. */
export const ensureHostConnection = registerHostCredential;

/** Poll until hub reports online or timeout (daemon dials `/ws/host` after apply). */
export async function waitForCloudOnline(
  isOnline: () => Promise<boolean>,
  opts?: { timeoutMs?: number; intervalMs?: number },
): Promise<boolean> {
  const timeoutMs = opts?.timeoutMs ?? 20_000;
  const intervalMs = opts?.intervalMs ?? 500;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await isOnline()) return true;
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  return isOnline();
}
