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

let provider: CloudAuthProvider | null = null;

/** Wire account-store (or tests) as the sole auth source for shared transport. */
export function registerCloudAuthProvider(next: CloudAuthProvider): void {
  provider = next;
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
