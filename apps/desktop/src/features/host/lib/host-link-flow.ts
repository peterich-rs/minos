/**
 * Pure Host Link orchestration (D02 §6 Desktop steps 2–8).
 *
 * Ports are injectable so unit tests mock daemon + cloud without Tauri/network.
 *
 * Flow:
 *   prepare_link → sign_link_proof → POST /v1/hosts/link → apply_link_token
 */

export type PrepareLinkResult = {
  installationId: string;
  publicKey: string;
  nonce: string;
};

export type SignLinkProofResult = {
  signature: string;
};

export type ApplyLinkTokenResult = {
  linked: boolean;
};

export type CloudLinkResult = {
  hostInstallationId: string;
  hostInstallationToken: string;
  pairId: string;
  accountId: string;
  hostDisplayName: string;
  linkedAtMs: number;
};

export type HostLinkPorts = {
  prepareLink: () => Promise<PrepareLinkResult>;
  signLinkProof: (
    installationId: string,
    nonce: string,
  ) => Promise<SignLinkProofResult>;
  applyLinkToken: (token: string) => Promise<ApplyLinkTokenResult>;
  linkHost: (input: {
    installationId: string;
    nonce: string;
    publicKey: string;
    signature: string;
    hostDisplayName: string;
  }) => Promise<CloudLinkResult>;
};

export type HostLinkSuccess = {
  linked: true;
  hostInstallationId: string;
  hostDisplayName: string;
  linkedAtMs: number;
  pairId: string;
  accountId: string;
};

export type HostLinkFailure = {
  linked: false;
  stage: "prepare" | "sign" | "cloud" | "apply";
  message: string;
  cause?: unknown;
};

export type HostLinkOutcome = HostLinkSuccess | HostLinkFailure;

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return String(error);
}

/**
 * Run the full same-account Host Link sequence.
 * Stops at the first failing stage and reports which step failed.
 */
export async function runHostLinkFlow(
  ports: HostLinkPorts,
  hostDisplayName: string,
): Promise<HostLinkOutcome> {
  const displayName = hostDisplayName.trim() || "This Mac";

  let prepared: PrepareLinkResult;
  try {
    prepared = await ports.prepareLink();
  } catch (error) {
    return {
      linked: false,
      stage: "prepare",
      message: errorMessage(error),
      cause: error,
    };
  }

  let signed: SignLinkProofResult;
  try {
    signed = await ports.signLinkProof(
      prepared.installationId,
      prepared.nonce,
    );
  } catch (error) {
    return {
      linked: false,
      stage: "sign",
      message: errorMessage(error),
      cause: error,
    };
  }

  let cloud: CloudLinkResult;
  try {
    cloud = await ports.linkHost({
      installationId: prepared.installationId,
      nonce: prepared.nonce,
      publicKey: prepared.publicKey,
      signature: signed.signature,
      hostDisplayName: displayName,
    });
  } catch (error) {
    return {
      linked: false,
      stage: "cloud",
      message: errorMessage(error),
      cause: error,
    };
  }

  try {
    const applied = await ports.applyLinkToken(cloud.hostInstallationToken);
    if (!applied.linked) {
      return {
        linked: false,
        stage: "apply",
        message: "Daemon did not accept host installation token",
      };
    }
  } catch (error) {
    return {
      linked: false,
      stage: "apply",
      message: errorMessage(error),
      cause: error,
    };
  }

  return {
    linked: true,
    hostInstallationId: cloud.hostInstallationId,
    hostDisplayName: cloud.hostDisplayName || displayName,
    linkedAtMs: cloud.linkedAtMs,
    pairId: cloud.pairId,
    accountId: cloud.accountId,
  };
}

export type UnlinkPorts = {
  unlinkHost: (hostInstallationId: string) => Promise<void>;
};

export type UnlinkOutcome =
  | { ok: true }
  | { ok: false; message: string; cause?: unknown };

export async function runHostUnlinkFlow(
  ports: UnlinkPorts,
  hostInstallationId: string,
): Promise<UnlinkOutcome> {
  const id = hostInstallationId.trim();
  if (!id) {
    return { ok: false, message: "Missing host installation id" };
  }
  try {
    await ports.unlinkHost(id);
    return { ok: true };
  } catch (error) {
    return { ok: false, message: errorMessage(error), cause: error };
  }
}
