/**
 * Desktop account session + automatic cloud (host) connection.
 *
 * Product rule: signing into Desktop on this Mac owns host control.
 * After auth, we silently bind + dial `/ws/host` (runtime). Account IM uses
 * `/ws/client` (im-hub-bridge). Product Online is Account sync primary;
 * Host readiness is secondary — never Link / Unlink.
 */

import { create } from "zustand";
import {
  clearStoredSession,
  EMPTY_HOST_BIND,
  ensureDesktopDeviceId,
  isAccessTokenFresh,
  loadStoredHostBind,
  loadStoredSession,
  saveStoredHostBind,
  saveStoredSession,
  sessionFromAuthResponse,
  type HostBindState,
  type MinosSession,
} from "@/shared/lib/account-session";
import {
  backendHttpBase,
  exchangeSupabaseSession,
  linkHost as cloudLinkHost,
  logoutSession,
  refreshSession,
} from "@/shared/lib/minos-cloud";
import {
  isSupabaseConfigured,
  signInWithSupabasePassword,
  signOutSupabase,
  signUpWithSupabasePassword,
} from "@/shared/lib/supabase";
import {
  cloudModeFromAccountSync,
  PROJECT_HOST_THIS_MAC,
  type CloudMode,
} from "@/shared/lib/host-status";
import { daemonApi } from "@/shared/lib/daemon";
import {
  registerHostCredential,
  waitForHubOnline,
} from "@/features/host/lib/ensure-host-connection";
import type { DesktopAuthPhase } from "@/shared/lib/desktop-root-gate";

export type AuthMode = "login" | "register";

export type CloudConnectionStatus = CloudMode;

type AccountState = {
  deviceId: string;
  session: MinosSession | null;
  /** Internal bind snapshot (diagnostics); not a product "Link" flag. */
  hostBind: HostBindState;
  /**
   * Host runtime readiness (`/ws/host`): online | connecting | offline | unknown.
   * Secondary signal — bot execution on this Mac. Not product primary Online.
   */
  cloudStatus: CloudConnectionStatus;
  /**
   * Account IM sync (`/ws/client`): primary product Online for send/receive.
   * Driven by im-hub-bridge HubRealtimeSyncState.
   */
  accountSyncStatus: CloudConnectionStatus;
  cloudError: string | null;
  authMode: AuthMode;
  authPhase: DesktopAuthPhase;
  busy: boolean;
  error: string | null;
  hydrated: boolean;

  setAuthMode: (mode: AuthMode) => void;
  clearError: () => void;

  hydrateAuth: () => Promise<void>;
  signIn: (email: string, password: string) => Promise<boolean>;
  signUp: (email: string, password: string) => Promise<boolean>;
  signOut: () => Promise<void>;

  /**
   * Connect to server as this Mac's host. Safe to call often:
   * - already Online → no-op
   * - has hit_ → only wait for dial (never re-register)
   * - no hit_ → one silent register
   * - `forceReregister` → mint new hit_ (Retry / recovery only)
   */
  ensureCloudConnection: (opts?: { forceReregister?: boolean }) => Promise<void>;
  /** Banner Retry: force re-register credential then dial. */
  retryCloudConnection: () => Promise<void>;

  /** Sync Host readiness from latest daemon hubOnline (polling). */
  syncCloudFromHub: (hubOnline: boolean | undefined) => void;
  /** Sync Account IM Online from `/ws/client` realtime state. */
  syncAccountFromHub: (
    state: "disconnected" | "connecting" | "syncing" | "live" | "error" | string,
  ) => void;

  isCloudConfigured: () => boolean;
  isSupabaseReady: () => boolean;
};

function isCloudConfigured(): boolean {
  try {
    const base = backendHttpBase();
    return Boolean(base.trim());
  } catch {
    return false;
  }
}

let ensureInFlight: Promise<void> | null = null;
let ensureGeneration = 0;

type DaemonCloudFlags = {
  hubOnline: boolean;
  hasHostToken: boolean;
};

async function refreshDaemonCloudFlags(): Promise<DaemonCloudFlags> {
  try {
    const { useWorkspaceStore } = await import("@/store/workspace-store");
    await useWorkspaceStore.getState().refreshDaemonStatus();
    const connection = useWorkspaceStore.getState().connection;
    return {
      hubOnline: connection?.hubOnline === true,
      hasHostToken: connection?.hasHostToken === true,
    };
  } catch {
    return { hubOnline: false, hasHostToken: false };
  }
}

async function refreshHubOnlineFlag(): Promise<boolean> {
  return (await refreshDaemonCloudFlags()).hubOnline;
}

function applyAuthSuccess(
  set: (
    partial:
      | Partial<AccountState>
      | ((state: AccountState) => Partial<AccountState>),
  ) => void,
  get: () => AccountState,
  session: MinosSession,
): void {
  saveStoredSession(session);
  const hostBind = loadStoredHostBind(session.accountId);
  set({
    session,
    hostBind,
    cloudStatus: "connecting",
    accountSyncStatus: "connecting",
    cloudError: null,
    authPhase: "authenticated",
    busy: false,
    error: null,
    hydrated: true,
  });
  void import("@/shared/lib/im-hub-bridge").then(({ ensureImHubBridge }) =>
    ensureImHubBridge(),
  );
  void get().ensureCloudConnection();
}

export const useAccountStore = create<AccountState>()((set, get) => ({
  deviceId: ensureDesktopDeviceId(),
  session: null,
  hostBind: { ...EMPTY_HOST_BIND },
  cloudStatus: "unknown",
  accountSyncStatus: "unknown",
  cloudError: null,
  authMode: "login",
  authPhase: "booting",
  busy: false,
  error: null,
  hydrated: false,

  setAuthMode: (authMode) => set({ authMode, error: null }),
  clearError: () => set({ error: null }),

  isCloudConfigured,
  isSupabaseReady: () => isSupabaseConfigured(),

  hydrateAuth: async () => {
    set({ authPhase: "booting", error: null });
    const stored = loadStoredSession();
    if (!stored) {
      set({
        session: null,
        hostBind: { ...EMPTY_HOST_BIND },
        cloudStatus: "unknown",
        accountSyncStatus: "unknown",
        cloudError: null,
        authPhase: "unauthenticated",
        hydrated: true,
      });
      return;
    }

    if (isAccessTokenFresh(stored)) {
      set({
        session: stored,
        hostBind: loadStoredHostBind(stored.accountId),
        cloudStatus: "connecting",
        accountSyncStatus: "connecting",
        cloudError: null,
        authPhase: "authenticated",
        hydrated: true,
      });
      void import("@/shared/lib/im-hub-bridge").then(({ ensureImHubBridge }) =>
        ensureImHubBridge(),
      );
      void get().ensureCloudConnection();
      return;
    }

    try {
      const tokens = await refreshSession(
        get().deviceId,
        stored.refreshToken,
      );
      const session: MinosSession = {
        ...stored,
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token,
        expiresInSec: tokens.expires_in,
        issuedAtMs: Date.now(),
      };
      applyAuthSuccess(set, get, session);
    } catch {
      clearStoredSession();
      set({
        session: null,
        hostBind: { ...EMPTY_HOST_BIND },
        cloudStatus: "unknown",
        accountSyncStatus: "unknown",
        cloudError: null,
        authPhase: "unauthenticated",
        hydrated: true,
        error: null,
      });
    }
  },

  signIn: async (email, password) => {
    set({ busy: true, error: null });
    try {
      if (!isSupabaseConfigured()) {
        throw new Error(
          "Supabase is required (set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY).",
        );
      }
      const deviceId = get().deviceId;
      const supabaseToken = await signInWithSupabasePassword(email, password);
      const auth = await exchangeSupabaseSession(deviceId, supabaseToken);
      applyAuthSuccess(set, get, sessionFromAuthResponse(auth));
      return true;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      set({ busy: false, error: message });
      return false;
    }
  },

  signUp: async (email, password) => {
    set({ busy: true, error: null });
    try {
      if (!isSupabaseConfigured()) {
        throw new Error(
          "Supabase is required (set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY).",
        );
      }
      const deviceId = get().deviceId;
      const supabaseToken = await signUpWithSupabasePassword(email, password);
      const auth = await exchangeSupabaseSession(deviceId, supabaseToken);
      applyAuthSuccess(set, get, sessionFromAuthResponse(auth));
      return true;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      set({ busy: false, error: message });
      return false;
    }
  },

  signOut: async () => {
    const { session, deviceId } = get();
    set({ busy: true, error: null });
    ensureGeneration += 1;
    if (session) {
      try {
        await logoutSession(deviceId, session.accessToken, session.refreshToken);
      } catch {
        // best-effort revoke
      }
    }
    await signOutSupabase();
    clearStoredSession();
    void import("@/shared/lib/im-hub-bridge").then(({ stopImHubBridge }) =>
      stopImHubBridge(),
    );
    set({
      session: null,
      hostBind: { ...EMPTY_HOST_BIND },
      cloudStatus: "unknown",
      accountSyncStatus: "unknown",
      cloudError: null,
      authPhase: "unauthenticated",
      busy: false,
      error: null,
      hydrated: true,
    });
  },

  ensureCloudConnection: async (opts) => {
    if (ensureInFlight && !opts?.forceReregister) return ensureInFlight;

    const forceReregister = opts?.forceReregister === true;

    const run = async () => {
      const gen = ++ensureGeneration;
      const session = get().session;
      if (!session) {
        set({ cloudStatus: "unknown", cloudError: null });
        return;
      }
      if (!isCloudConfigured()) {
        set({
          cloudStatus: "offline",
          cloudError: "Backend URL not configured",
        });
        return;
      }

      set({ cloudStatus: "connecting", cloudError: null });

      // ── Steady state: never re-register if we already have hit_ ────────
      // Product has no "Link". Protocol only needs hit_ once. Calling
      // POST /v1/hosts/link again revokes the live token → 401 races.
      const flags = await refreshDaemonCloudFlags();
      if (gen !== ensureGeneration) return;

      if (flags.hubOnline) {
        set({ cloudStatus: "online", cloudError: null });
        return;
      }

      if (!forceReregister && flags.hasHostToken) {
        // Credential exists — only wait for dialer; do not mint/revoke.
        const online = await waitForHubOnline(refreshHubOnlineFlag, {
          timeoutMs: 20_000,
          intervalMs: 500,
        });
        if (gen !== ensureGeneration) return;
        if (online) {
          set({ cloudStatus: "online", cloudError: null });
          return;
        }
        // Still offline with a local hit_: backend down or token dead.
        // Do **not** auto-rotate here (would thrash). Retry uses forceReregister.
        set({
          cloudStatus: "offline",
          cloudError:
            "Has local host credential but server is unreachable. Check minos-backend, then Retry.",
        });
        return;
      }

      // ── Missing hit_ or explicit Retry: one silent register ────────────
      const outcome = await registerHostCredential(
        {
          prepareLink: () => daemonApi.hostPrepareLink(),
          signLinkProof: (installationId, nonce) =>
            daemonApi.hostSignLinkProof(installationId, nonce),
          applyLinkToken: (token) => daemonApi.hostApplyLinkToken(token),
          registerHost: (input) =>
            cloudLinkHost(get().deviceId, session.accessToken, input),
        },
        PROJECT_HOST_THIS_MAC,
      );

      if (gen !== ensureGeneration) return;

      if (!outcome.ok) {
        set({
          cloudStatus: "offline",
          cloudError: `Connect failed (${outcome.stage}): ${outcome.message}`,
        });
        return;
      }

      const hostBind: HostBindState = {
        bound: true,
        hostInstallationId: outcome.hostInstallationId,
        hostDisplayName: outcome.hostDisplayName,
        boundAtMs: outcome.linkedAtMs,
        pairId: outcome.pairId,
      };
      saveStoredHostBind(session.accountId, hostBind);
      set({ hostBind });

      const online = await waitForHubOnline(refreshHubOnlineFlag, {
        timeoutMs: 20_000,
        intervalMs: 500,
      });

      if (gen !== ensureGeneration) return;

      if (online) {
        set({ cloudStatus: "online", cloudError: null });
      } else {
        set({
          cloudStatus: "offline",
          cloudError:
            "Signed in but not connected to the server yet. Retry or check that minos-backend is running.",
        });
      }
    };

    ensureInFlight = run().finally(() => {
      ensureInFlight = null;
    });
    return ensureInFlight;
  },

  retryCloudConnection: async () => {
    ensureGeneration += 1;
    ensureInFlight = null;
    await get().ensureCloudConnection({ forceReregister: true });
  },

  syncCloudFromHub: (hubOnline) => {
    const { session, cloudStatus } = get();
    if (!session) return;
    // Do not clobber an in-flight ensure.
    if (cloudStatus === "connecting") return;
    if (hubOnline === true) {
      if (cloudStatus !== "online") {
        set({ cloudStatus: "online", cloudError: null });
      }
      return;
    }
    if (hubOnline === false && cloudStatus === "online") {
      set({
        cloudStatus: "offline",
        cloudError: "Disconnected from server",
      });
    }
  },

  syncAccountFromHub: (state) => {
    const { session } = get();
    if (!session) {
      if (get().accountSyncStatus !== "unknown") {
        set({ accountSyncStatus: "unknown" });
      }
      return;
    }
    const next = cloudModeFromAccountSync(state);
    if (get().accountSyncStatus !== next) {
      set({ accountSyncStatus: next });
    }
  },
}));
