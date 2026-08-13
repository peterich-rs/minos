/**
 * Process-local cloud auth snapshot for Hub transport.
 *
 * Registered once from account-store so pure shared/lib transport helpers
 * never import Zustand stores. Ownership of the session remains account-store.
 */

export type CloudAuthSnapshot = {
  deviceId: string;
  accessToken: string;
  accountId: string;
  authPhase: string;
};

type CloudAuthProvider = () => CloudAuthSnapshot | null;
/** Refresh access token; returns new bearer or null if refresh failed / signed out. */
type CloudAccessTokenRefresher = () => Promise<string | null>;

let provider: CloudAuthProvider | null = null;
let refresher: CloudAccessTokenRefresher | null = null;
let refreshInFlight: Promise<string | null> | null = null;

/** Wire account-store (or tests) as the sole auth source for shared transport. */
export function registerCloudAuthProvider(next: CloudAuthProvider): void {
  provider = next;
}

/** Wire account-store proactive / 401 refresh implementation. */
export function registerCloudAccessTokenRefresher(
  next: CloudAccessTokenRefresher,
): void {
  refresher = next;
}

/** Current Hub credentials, or null when signed out / incomplete. */
export function getCloudAuth(): CloudAuthSnapshot | null {
  if (!provider) return null;
  try {
    return provider();
  } catch {
    return null;
  }
}

/** True when authenticated with a usable access token (Hub IM mode gate). */
export function isCloudAuthReady(): boolean {
  const auth = getCloudAuth();
  return Boolean(
    auth?.accessToken?.trim() &&
      auth.accountId.trim() &&
      auth.authPhase === "authenticated",
  );
}

/**
 * Ensure a fresh access token (single-flight). Used by HTTP 401 retry and
 * proactive refresh. Returns null when signed out or refresh fails.
 */
export async function ensureFreshCloudAccessToken(): Promise<string | null> {
  if (!refresher) {
    return getCloudAuth()?.accessToken?.trim() || null;
  }
  if (!refreshInFlight) {
    refreshInFlight = (async () => {
      try {
        return await refresher!();
      } finally {
        refreshInFlight = null;
      }
    })();
  }
  return refreshInFlight;
}
