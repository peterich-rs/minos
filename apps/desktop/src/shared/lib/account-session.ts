/**
 * Desktop account session types + local persistence.
 *
 * Dual-session model (D01):
 * - Supabase session: IdP UX only (managed by @supabase/supabase-js when configured)
 * - Minos session: access/refresh tokens for all product API (this module)
 *
 * Host bind cache is **account-scoped** and internal only (auto-connect after
 * login). Users never manage Link/Unlink in the UI.
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

/** Internal bind snapshot for this Mac under one Minos account. */
export type HostBindState = {
  bound: boolean;
  hostInstallationId: string | null;
  hostDisplayName: string | null;
  boundAtMs: number | null;
  pairId: string | null;
};

export const EMPTY_HOST_BIND: HostBindState = {
  bound: false,
  hostInstallationId: null,
  hostDisplayName: null,
  boundAtMs: null,
  pairId: null,
};

/** @deprecated Use HostBindState / EMPTY_HOST_BIND */
export type HostLinkState = HostBindState;
/** @deprecated */
export const EMPTY_HOST_LINK = EMPTY_HOST_BIND;

const SESSION_KEY = "minos.desktop.session";
const DEVICE_ID_KEY = "minos.desktop.device-id";
/** Account-scoped map: `{ [accountId]: HostBindState }`. */
const HOST_BINDS_KEY = "minos.desktop.host-binds";
/** Legacy keys (dropped on access). */
const LEGACY_HOST_LINKS_KEY = "minos.desktop.host-links";
const LEGACY_HOST_LINK_KEY = "minos.desktop.host-link";

function storageAvailable(): boolean {
  return (
    typeof window !== "undefined" && typeof window.localStorage !== "undefined"
  );
}

function normalizeAccountId(accountId: string | null | undefined): string | null {
  const trimmed = accountId?.trim();
  return trimmed ? trimmed : null;
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

function parseHostBind(raw: unknown): HostBindState | null {
  if (!raw || typeof raw !== "object") return null;
  const parsed = raw as Record<string, unknown>;
  // Accept legacy { linked } shape.
  const bound = parsed.bound === true || parsed.linked === true;
  const hostInstallationId =
    typeof parsed.hostInstallationId === "string"
      ? parsed.hostInstallationId
      : null;
  const hostDisplayName =
    typeof parsed.hostDisplayName === "string" ? parsed.hostDisplayName : null;
  const boundAtMs =
    typeof parsed.boundAtMs === "number"
      ? parsed.boundAtMs
      : typeof parsed.linkedAtMs === "number"
        ? parsed.linkedAtMs
        : null;
  const pairId = typeof parsed.pairId === "string" ? parsed.pairId : null;
  return {
    bound,
    hostInstallationId,
    hostDisplayName,
    boundAtMs,
    pairId,
  };
}

function dropLegacyHostKeys(): void {
  if (!storageAvailable()) return;
  window.localStorage.removeItem(LEGACY_HOST_LINK_KEY);
  window.localStorage.removeItem(LEGACY_HOST_LINKS_KEY);
}

function readHostBindsMap(): Record<string, HostBindState> {
  if (!storageAvailable()) return {};
  dropLegacyHostKeys();
  const raw = window.localStorage.getItem(HOST_BINDS_KEY);
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      window.localStorage.removeItem(HOST_BINDS_KEY);
      return {};
    }
    const out: Record<string, HostBindState> = {};
    for (const [key, value] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      const accountId = normalizeAccountId(key);
      const bind = parseHostBind(value);
      if (accountId && bind) out[accountId] = bind;
    }
    return out;
  } catch {
    window.localStorage.removeItem(HOST_BINDS_KEY);
    return {};
  }
}

function writeHostBindsMap(map: Record<string, HostBindState>): void {
  if (!storageAvailable()) return;
  if (Object.keys(map).length === 0) {
    window.localStorage.removeItem(HOST_BINDS_KEY);
    return;
  }
  window.localStorage.setItem(HOST_BINDS_KEY, JSON.stringify(map));
}

export function loadStoredHostBind(
  accountId: string | null | undefined,
): HostBindState {
  const id = normalizeAccountId(accountId);
  if (!id) return { ...EMPTY_HOST_BIND };
  const map = readHostBindsMap();
  const bind = map[id];
  return bind ? { ...bind } : { ...EMPTY_HOST_BIND };
}

/** @deprecated Use loadStoredHostBind */
export function loadStoredHostLink(
  accountId: string | null | undefined,
): HostBindState {
  return loadStoredHostBind(accountId);
}

export function saveStoredHostBind(
  accountId: string,
  bind: HostBindState,
): void {
  const id = normalizeAccountId(accountId);
  if (!id || !storageAvailable()) return;
  const map = readHostBindsMap();
  map[id] = {
    bound: bind.bound === true,
    hostInstallationId: bind.hostInstallationId,
    hostDisplayName: bind.hostDisplayName,
    boundAtMs: bind.boundAtMs,
    pairId: bind.pairId,
  };
  writeHostBindsMap(map);
}

/** @deprecated Use saveStoredHostBind */
export function saveStoredHostLink(
  accountId: string,
  link: HostBindState,
): void {
  saveStoredHostBind(accountId, {
    bound: link.bound === true || (link as { linked?: boolean }).linked === true,
    hostInstallationId: link.hostInstallationId,
    hostDisplayName: link.hostDisplayName,
    boundAtMs:
      link.boundAtMs ??
      (link as { linkedAtMs?: number | null }).linkedAtMs ??
      null,
    pairId: link.pairId,
  });
}

export function clearStoredHostBind(accountId: string): void {
  const id = normalizeAccountId(accountId);
  if (!id || !storageAvailable()) return;
  const map = readHostBindsMap();
  if (!(id in map)) {
    dropLegacyHostKeys();
    return;
  }
  delete map[id];
  writeHostBindsMap(map);
}

/** @deprecated Use clearStoredHostBind */
export function clearStoredHostLink(accountId: string): void {
  clearStoredHostBind(accountId);
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
