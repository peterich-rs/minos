/**
 * Single-flight ensure for CloudDigestCache hydrate.
 */

import { cloudDigestCache } from "@/shared/lib/cloud-digest-cache";
import { listCloudConversations } from "@/shared/lib/minos-cloud";
import { useAccountStore } from "@/store/account-store";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";

let hydrateInFlight: Promise<void> | null = null;
let hydrateGeneration = 0;

/** Cancel in-flight digest hydrate (account leave). */
export function cancelCloudDigestHydrate(): void {
  hydrateGeneration += 1;
  hydrateInFlight = null;
}

/** Hydrate CloudDigestCache once (or after invalidate). Concurrent callers share one request. */
export async function ensureCloudDigestHydrated(
  opts?: { force?: boolean },
): Promise<void> {
  const { deviceId, session, authPhase } = useAccountStore.getState();
  const accountId = session?.accountId?.trim() ?? "";
  if (
    !isCloudImMode({
      authPhase,
      accessToken: session?.accessToken,
    }) ||
    !session?.accessToken ||
    !accountId
  ) {
    return;
  }

  if (!opts?.force && cloudDigestCache.isHydratedFor(accountId)) {
    return;
  }

  // Force after concurrent hydrate: wait then re-query so SnapshotRequired
  // never leaves a stale cache behind a no-op await.
  if (hydrateInFlight) {
    await hydrateInFlight;
    if (!opts?.force) {
      return;
    }
  }

  if (!opts?.force && cloudDigestCache.isHydratedFor(accountId)) {
    return;
  }

  const gen = hydrateGeneration;
  hydrateInFlight = (async () => {
    try {
      if (opts?.force) {
        cloudDigestCache.invalidate();
      }
      const digests = await listCloudConversations(
        deviceId,
        session.accessToken,
      );
      if (gen !== hydrateGeneration) return;
      const still = useAccountStore.getState().session?.accountId?.trim() ?? "";
      if (still !== accountId) return;
      cloudDigestCache.hydrate(digests, accountId);
    } finally {
      if (gen === hydrateGeneration) {
        hydrateInFlight = null;
      }
    }
  })();

  await hydrateInFlight;
}
