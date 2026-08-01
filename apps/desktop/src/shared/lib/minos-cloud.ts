/**
 * Minos backend HTTP client for Desktop account session + Host Link.
 *
 * Mirrors `apps/web/src/lib/minos.ts` auth/host paths (D01 dual session,
 * D02 same-account host link). Device role is `desktop-console`.
 */

const DEFAULT_BACKEND_URL = "http://127.0.0.1:8787";

export type AuthResponse = {
  account: { account_id: string; email: string };
  access_token: string;
  refresh_token: string;
  expires_in: number;
};

export type HostLinkResult = {
  hostInstallationId: string;
  hostInstallationToken: string;
  pairId: string;
  accountId: string;
  hostDisplayName: string;
  linkedAtMs: number;
};

export type ListedHost = {
  hostInstallationId: string;
  hostDisplayName: string;
  linkedAtMs: number;
  online: boolean;
};

export class MinosCloudError extends Error {
  readonly status: number;
  readonly code: string | null;
  readonly payload: unknown;

  constructor(
    message: string,
    status: number,
    code: string | null = null,
    payload?: unknown,
  ) {
    super(message);
    this.name = "MinosCloudError";
    this.status = status;
    this.code = code;
    this.payload = payload;
  }
}

export function backendHttpBase(): string {
  const raw =
    (import.meta.env.VITE_MINOS_BACKEND_URL as string | undefined) ??
    DEFAULT_BACKEND_URL;
  return raw.replace(/\/+$/, "");
}

function deviceHeaders(
  deviceId: string,
  accessToken?: string,
): Record<string, string> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "x-device-id": deviceId,
    "x-device-role": "desktop-console",
    "x-device-name": "Minos Desktop",
  };
  if (accessToken) {
    headers.authorization = `Bearer ${accessToken}`;
  }
  return headers;
}

async function parseErrorPayload(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null;
  }
}

function errorFromPayload(
  status: number,
  payload: unknown,
): MinosCloudError {
  if (payload && typeof payload === "object") {
    const record = payload as Record<string, unknown>;
    const nested = record.error;
    if (nested && typeof nested === "object") {
      const nestedRecord = nested as Record<string, unknown>;
      const code =
        typeof nestedRecord.code === "string" ? nestedRecord.code : null;
      const message =
        typeof nestedRecord.message === "string"
          ? nestedRecord.message
          : code ?? `request failed (${status})`;
      return new MinosCloudError(message, status, code, payload);
    }
    if (typeof record.message === "string") {
      return new MinosCloudError(record.message, status, null, payload);
    }
  }
  return new MinosCloudError(`request failed (${status})`, status, null, payload);
}

async function requestJson<T>(
  path: string,
  init: RequestInit,
): Promise<T> {
  const response = await fetch(`${backendHttpBase()}${path}`, init);
  if (!response.ok) {
    const payload = await parseErrorPayload(response);
    throw errorFromPayload(response.status, payload);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return response.json() as Promise<T>;
}

/** Exchange Supabase access token → Minos AuthResp (same shape as login). */
export async function exchangeSupabaseSession(
  deviceId: string,
  supabaseAccessToken: string,
  deviceName = "Minos Desktop",
): Promise<AuthResponse> {
  return requestJson<AuthResponse>("/v1/auth/supabase", {
    method: "POST",
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({
      access_token: supabaseAccessToken,
      device_name: deviceName,
    }),
  });
}

export async function refreshSession(
  deviceId: string,
  refreshToken: string,
): Promise<{
  access_token: string;
  refresh_token: string;
  expires_in: number;
}> {
  return requestJson("/v1/auth/refresh", {
    method: "POST",
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({ refresh_token: refreshToken }),
  });
}

/** Revoke Minos refresh token. Best-effort callers should catch errors. */
export async function logoutSession(
  deviceId: string,
  accessToken: string,
  refreshToken: string,
): Promise<void> {
  await requestJson<void>("/v1/auth/logout", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ refresh_token: refreshToken }),
  });
}

type ResponseEnvelope<T> = { data: T };

export async function linkHost(
  deviceId: string,
  accessToken: string,
  body: {
    installationId: string;
    nonce: string;
    publicKey: string;
    signature: string;
    hostDisplayName: string;
  },
): Promise<HostLinkResult> {
  const envelope = await requestJson<
    ResponseEnvelope<{
      host_installation_id: string;
      host_installation_token: string;
      link: {
        pair_id: string;
        account_id: string;
        host_display_name: string;
        linked_at_ms: number;
      };
    }>
  >("/v1/hosts/link", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({
      installation_id: body.installationId,
      nonce: body.nonce,
      public_key: body.publicKey,
      signature: body.signature,
      host_display_name: body.hostDisplayName,
    }),
  });
  const data = envelope.data;
  return {
    hostInstallationId: data.host_installation_id,
    hostInstallationToken: data.host_installation_token,
    pairId: data.link.pair_id,
    accountId: data.link.account_id,
    hostDisplayName: data.link.host_display_name,
    linkedAtMs: data.link.linked_at_ms,
  };
}

export async function unlinkHost(
  deviceId: string,
  accessToken: string,
  hostInstallationId: string,
): Promise<void> {
  await requestJson<void>("/v1/hosts/unlink", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ host_installation_id: hostInstallationId }),
  });
}

export async function listHosts(
  deviceId: string,
  accessToken: string,
): Promise<ListedHost[]> {
  const envelope = await requestJson<
    ResponseEnvelope<{
      hosts: Array<{
        host_installation_id: string;
        host_display_name: string;
        linked_at_ms: number;
        online: boolean;
      }>;
    }>
  >("/v1/hosts", {
    method: "GET",
    headers: deviceHeaders(deviceId, accessToken),
  });
  return envelope.data.hosts.map((h) => ({
    hostInstallationId: h.host_installation_id,
    hostDisplayName: h.host_display_name,
    linkedAtMs: h.linked_at_ms,
    online: h.online,
  }));
}
