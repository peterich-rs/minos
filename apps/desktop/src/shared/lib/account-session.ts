/**
 * Desktop account session types + local persistence.
 *
 * Dual-session model (D01):
 * - Supabase session: IdP UX only (managed by @supabase/supabase-js when configured)
 * - Minos session: access/refresh tokens for all product API (this module)
 *
 * Storage: app-scoped localStorage. Tokens never ship in git; keys are
 * namespaced under `minos.desktop.*`. Device id is a stable UUID for
 * `X-Device-Id` / `desktop-console` installation binding.
 */

export type MinosSession = {
  accountId: string;
  email: string;
  accessToken: string;
  refreshToken: string;
  /** Epoch ms when access token was issued (client-side clock). */
  issuedAtMs: number;
  /** Access token lifetime from exchange/refresh (`expires_in` seconds). */
  expiresInSec: number;
};

/** Host Link state for this Mac (plane B). Independent of human login UX. */
export type HostLinkState = {
  linked: boolean;
  hostInstallationId: string | null;
  hostDisplayName: string | null;
  linkedAtMs: number | null;
  pairId: string | null;
};

export const EMPTY_HOST_LINK: HostLinkState = {
  linked: false,
  hostInstallationId: null,
  hostDisplayName: null,
  linkedAtMs: null,
  pairId: null,
};

const SESSION_KEY = "minos.desktop.session";
const DEVICE_ID_KEY = "minos.desktop.device-id";
const HOST_LINK_KEY = "minos.desktop.host-link";

function storageAvailable(): boolean {
  return (
    typeof window !== "undefined" && typeof window.localStorage !== "undefined"
  );
}

export function ensureDesktopDeviceId(): string {
  if (!storageAvailable()) {
    return "desktop-console-device";
  }
  const current = window.localStorage.getItem(DEVICE_ID_KEY);
  if (current?.trim()) {
    return current.trim();
  }
  const next =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `desktop-${Date.now().toString(36)}`;
  window.localStorage.setItem(DEVICE_ID_KEY, next);
  return next;
}

export function loadStoredSession(): MinosSession | null {
  if (!storageAvailable()) return null;
  const raw = window.localStorage.getItem(SESSION_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<MinosSession>;
    if (
      typeof parsed.accountId !== "string" ||
      typeof parsed.accessToken !== "string" ||
      typeof parsed.refreshToken !== "string"
    ) {
      window.localStorage.removeItem(SESSION_KEY);
      return null;
    }
    return {
      accountId: parsed.accountId,
      email: typeof parsed.email === "string" ? parsed.email : "",
      accessToken: parsed.accessToken,
      refreshToken: parsed.refreshToken,
      issuedAtMs:
        typeof parsed.issuedAtMs === "number" ? parsed.issuedAtMs : Date.now(),
      expiresInSec:
        typeof parsed.expiresInSec === "number" ? parsed.expiresInSec : 900,
    };
  } catch {
    window.localStorage.removeItem(SESSION_KEY);
    return null;
  }
}

export function saveStoredSession(session: MinosSession): void {
  if (!storageAvailable()) return;
  window.localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

export function clearStoredSession(): void {
  if (!storageAvailable()) return;
  window.localStorage.removeItem(SESSION_KEY);
}

export function loadStoredHostLink(): HostLinkState {
  if (!storageAvailable()) return { ...EMPTY_HOST_LINK };
  const raw = window.localStorage.getItem(HOST_LINK_KEY);
  if (!raw) return { ...EMPTY_HOST_LINK };
  try {
    const parsed = JSON.parse(raw) as Partial<HostLinkState>;
    return {
      linked: parsed.linked === true,
      hostInstallationId:
        typeof parsed.hostInstallationId === "string"
          ? parsed.hostInstallationId
          : null,
      hostDisplayName:
        typeof parsed.hostDisplayName === "string"
          ? parsed.hostDisplayName
          : null,
      linkedAtMs:
        typeof parsed.linkedAtMs === "number" ? parsed.linkedAtMs : null,
      pairId: typeof parsed.pairId === "string" ? parsed.pairId : null,
    };
  } catch {
    window.localStorage.removeItem(HOST_LINK_KEY);
    return { ...EMPTY_HOST_LINK };
  }
}

export function saveStoredHostLink(link: HostLinkState): void {
  if (!storageAvailable()) return;
  window.localStorage.setItem(HOST_LINK_KEY, JSON.stringify(link));
}

export function clearStoredHostLink(): void {
  if (!storageAvailable()) return;
  window.localStorage.removeItem(HOST_LINK_KEY);
}

/** True when access token is still usable with a 60s skew buffer. */
export function isAccessTokenFresh(
  session: MinosSession,
  nowMs: number = Date.now(),
): boolean {
  const expiresAt = session.issuedAtMs + session.expiresInSec * 1000;
  return nowMs < expiresAt - 60_000;
}

export function sessionFromAuthResponse(resp: {
  account: { account_id: string; email: string };
  access_token: string;
  refresh_token: string;
  expires_in: number;
  issuedAtMs?: number;
}): MinosSession {
  return {
    accountId: resp.account.account_id,
    email: resp.account.email ?? "",
    accessToken: resp.access_token,
    refreshToken: resp.refresh_token,
    issuedAtMs: resp.issuedAtMs ?? Date.now(),
    expiresInSec: resp.expires_in,
  };
}
