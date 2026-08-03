/**
 * Single-flight ensure for HubDigestCache hydrate.
 */

import { hubDigestCache } from "@/shared/lib/hub-digest-cache";
import { listHubConversations } from "@/shared/lib/minos-cloud";
import { useAccountStore } from "@/store/account-store";
import { isHubImMode } from "@/shared/lib/hub-timeline";

let hydrateInFlight: Promise<void> | null = null;

/** Hydrate HubDigestCache once (or after invalidate). Concurrent callers share one request. */
export async function ensureHubDigestHydrated(
  opts?: { force?: boolean },
): Promise<void> {
  const { deviceId, session, authPhase } = useAccountStore.getState();
  if (
    !isHubImMode({
      authPhase,
      accessToken: session?.accessToken,
    }) ||
    !session?.accessToken
  ) {
    return;
  }

  if (!opts?.force && hubDigestCache.isHydrated()) {
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

  if (!opts?.force && hubDigestCache.isHydrated()) {
    return;
  }

  hydrateInFlight = (async () => {
    try {
      if (opts?.force) {
        hubDigestCache.invalidate();
      }
      const digests = await listHubConversations(
        deviceId,
        session.accessToken,
      );
      hubDigestCache.hydrate(digests);
    } finally {
      hydrateInFlight = null;
    }
  })();

  await hydrateInFlight;
}
