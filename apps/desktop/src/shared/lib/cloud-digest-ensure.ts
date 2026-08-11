/**
 * Single-flight ensure for CloudDigestCache hydrate.
 */

import { cloudDigestCache } from "@/shared/lib/cloud-digest-cache";
import { listCloudConversations } from "@/shared/lib/minos-cloud";
import { useAccountStore } from "@/store/account-store";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";

let hydrateInFlight: Promise<void> | null = null;

/** Hydrate CloudDigestCache once (or after invalidate). Concurrent callers share one request. */
export async function ensureCloudDigestHydrated(
  opts?: { force?: boolean },
): Promise<void> {
  const { deviceId, session, authPhase } = useAccountStore.getState();
  if (
    !isCloudImMode({
      authPhase,
      accessToken: session?.accessToken,
    }) ||
    !session?.accessToken
  ) {
    return;
  }

  if (!opts?.force && cloudDigestCache.isHydrated()) {
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

  if (!opts?.force && cloudDigestCache.isHydrated()) {
    return;
  }

  hydrateInFlight = (async () => {
    try {
      if (opts?.force) {
        cloudDigestCache.invalidate();
      }
      const digests = await listCloudConversations(
        deviceId,
        session.accessToken,
      );
      cloudDigestCache.hydrate(digests);
    } finally {
      hydrateInFlight = null;
    }
  })();

  await hydrateInFlight;
}
