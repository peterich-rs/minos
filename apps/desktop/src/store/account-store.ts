/**
 * Desktop account session + automatic cloud (host) connection.
 *
 * Product rule: signing into Desktop on this Mac owns host control.
 * After auth, we silently bind + dial `/ws/host` (runtime). Account IM uses
 * `/ws/client` (im-cloud-bridge). Product Online is Account sync primary;
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
import { runHostEnsure } from "@/features/host/lib/host-connection-machine";
import type { DesktopAuthPhase } from "@/shared/lib/desktop-root-gate";
import { registerCloudAuthProvider } from "@/shared/lib/cloud-auth";
import {
  bumpAccountScopeGeneration,
  leaveAccountScope,
} from "@/store/leave-account-scope";
import { refreshDaemonCloudFlags } from "@/store/daemon-status-port";

export type AuthMode = "login" | "register";

export type CloudConnectionStatus = CloudMode;

type AccountState = {
  deviceId: string;
  session: MinosSession | null;
  /** Internal bind snapshot (diagnostics); not a product "Link" flag. */
  hostBind: HostBindState;
  /**
   * Account that owns the live host credential for this process.
   * Null after leave; Host online requires match with session.accountId.
   */
  hostCredentialAccountId: string | null;
  /**
   * Host runtime readiness (`/ws/host`): online | connecting | offline | unknown.
   * Secondary signal — bot execution on this Mac. Not product primary Online.
   */
  cloudStatus: CloudConnectionStatus;
  /**
   * Account IM sync (`/ws/client`): primary product Online for send/receive.
   * Driven by im-cloud-bridge CloudRealtimeSyncState.
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

  /** Sync Host readiness from latest daemon cloudOnline (polling). */
  syncCloudFromCloud: (cloudOnline: boolean | undefined) => void;
  /** Sync Account IM Online from `/ws/client` realtime state. */
  syncAccountFromCloud: (
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
let hydrateInFlight: Promise<void> | null = null;
let hydrateGeneration = 0;

function applyAuthSuccess(
  set: (
    partial:
      | Partial<AccountState>
      | ((state: AccountState) => Partial<AccountState>),
  ) => void,
  get: () => AccountState,
  session: MinosSession,
): void {
  const prevAccountId = get().session?.accountId?.trim() ?? null;
  const nextAccountId = session.accountId.trim();
  if (prevAccountId && prevAccountId !== nextAccountId) {
    ensureGeneration += 1;
    ensureInFlight = null;
    hydrateGeneration += 1;
    hydrateInFlight = null;
    leaveAccountScope("account-switch");
  }
  saveStoredSession(session);
  const hostBind = loadStoredHostBind(session.accountId);
  const hostOwned =
    get().hostCredentialAccountId === nextAccountId && hostBind.bound;
  set({
    session,
    hostBind,
    // New account (or unbound) must re-establish host ownership before online.
    hostCredentialAccountId: hostOwned ? nextAccountId : null,
    cloudStatus: "connecting",
    accountSyncStatus: "connecting",
    cloudError: null,
    authPhase: "authenticated",
    busy: false,
    error: null,
    hydrated: true,
  });
  void import("@/store/im/im-cloud-bridge").then(({ ensureImCloudBridge }) =>
    ensureImCloudBridge(),
  );
  const forceReregister =
    !hostOwned || prevAccountId !== nextAccountId || !hostBind.bound;
  void get().ensureCloudConnection(
    forceReregister ? { forceReregister: true } : undefined,
  );
}

export const useAccountStore = create<AccountState>()((set, get) => ({
  deviceId: ensureDesktopDeviceId(),
  session: null,
  hostBind: { ...EMPTY_HOST_BIND },
  hostCredentialAccountId: null,
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
    if (hydrateInFlight) return hydrateInFlight;
    const gen = ++hydrateGeneration;
    hydrateInFlight = (async () => {
      set({ authPhase: "booting", error: null });
      const stored = loadStoredSession();
      if (!stored) {
        if (gen !== hydrateGeneration) return;
        set({
          session: null,
          hostBind: { ...EMPTY_HOST_BIND },
          hostCredentialAccountId: null,
          cloudStatus: "unknown",
          accountSyncStatus: "unknown",
          cloudError: null,
          authPhase: "unauthenticated",
          hydrated: true,
        });
        return;
      }

      if (isAccessTokenFresh(stored)) {
        if (gen !== hydrateGeneration) return;
        const hostBind = loadStoredHostBind(stored.accountId);
        set({
          session: stored,
          hostBind,
          hostCredentialAccountId: hostBind.bound ? stored.accountId : null,
          cloudStatus: "connecting",
          accountSyncStatus: "connecting",
          cloudError: null,
          authPhase: "authenticated",
          hydrated: true,
        });
        void import("@/store/im/im-cloud-bridge").then(({ ensureImCloudBridge }) =>
          ensureImCloudBridge(),
        );
        void get().ensureCloudConnection(
          hostBind.bound ? undefined : { forceReregister: true },
        );
        return;
      }

      try {
        const tokens = await refreshSession(
          get().deviceId,
          stored.refreshToken,
        );
        if (gen !== hydrateGeneration) return;
        const session: MinosSession = {
          ...stored,
          accessToken: tokens.access_token,
          refreshToken: tokens.refresh_token,
          expiresInSec: tokens.expires_in,
          issuedAtMs: Date.now(),
        };
        applyAuthSuccess(set, get, session);
      } catch {
        if (gen !== hydrateGeneration) return;
        clearStoredSession();
        ensureGeneration += 1;
        ensureInFlight = null;
        leaveAccountScope("auth-invalid");
        set({
          session: null,
          hostBind: { ...EMPTY_HOST_BIND },
          hostCredentialAccountId: null,
          cloudStatus: "unknown",
          accountSyncStatus: "unknown",
          cloudError: null,
          authPhase: "unauthenticated",
          hydrated: true,
          error: null,
        });
      }
    })().finally(() => {
      if (gen === hydrateGeneration) {
        hydrateInFlight = null;
      }
    });
    return hydrateInFlight;
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
    ensureInFlight = null;
    hydrateGeneration += 1;
    hydrateInFlight = null;
    bumpAccountScopeGeneration();
    if (session) {
      try {
        await logoutSession(deviceId, session.accessToken, session.refreshToken);
      } catch {
        // best-effort revoke
      }
    }
    await signOutSupabase();
    clearStoredSession();
    leaveAccountScope("sign-out");
    set({
      session: null,
      hostBind: { ...EMPTY_HOST_BIND },
      hostCredentialAccountId: null,
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

      const accountId = session.accountId.trim();
      const result = await runHostEnsure(
        {
          forceReregister,
          sessionAccountId: accountId,
          hostCredentialAccountId: get().hostCredentialAccountId,
        },
        {
          refreshFlags: refreshDaemonCloudFlags,
          isOnline: async () =>
            (await refreshDaemonCloudFlags()).cloudOnline,
          registerPorts: {
            prepareLink: () => daemonApi.hostPrepareLink(),
            signLinkProof: (installationId, nonce) =>
              daemonApi.hostSignLinkProof(installationId, nonce),
            applyLinkToken: (token) => daemonApi.hostApplyLinkToken(token),
            registerHost: (input) =>
              cloudLinkHost(get().deviceId, session.accessToken, input),
          },
          hostDisplayName: PROJECT_HOST_THIS_MAC,
          waitOpts: { timeoutMs: 20_000, intervalMs: 500 },
        },
      );

      if (gen !== ensureGeneration) return;

      if (result.kind === "online") {
        set({ cloudStatus: "online", cloudError: null });
        return;
      }

      if (result.kind === "offline") {
        set({
          cloudStatus: "offline",
          cloudError: result.message,
        });
        return;
      }

      if (result.kind === "register-failed") {
        set({
          cloudStatus: "offline",
          hostCredentialAccountId: null,
          cloudError: `Connect failed (${result.stage}): ${result.message}`,
        });
        return;
      }

      const hostBind: HostBindState = {
        bound: true,
        hostInstallationId: result.hostInstallationId,
        hostDisplayName: result.hostDisplayName,
        boundAtMs: result.linkedAtMs,
        pairId: result.pairId,
      };
      saveStoredHostBind(session.accountId, hostBind);
      set({ hostBind, hostCredentialAccountId: accountId });

      if (result.kind === "registered-online") {
        set({ cloudStatus: "online", cloudError: null });
        return;
      }

      set({
        cloudStatus: "offline",
        cloudError: result.message,
      });
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

  syncCloudFromCloud: (cloudOnline) => {
    const { session, cloudStatus, hostCredentialAccountId } = get();
    if (!session) return;
    // Foreign or cleared host ownership must never paint Host online.
    if (hostCredentialAccountId !== session.accountId) return;
    // Do not clobber an in-flight ensure.
    if (cloudStatus === "connecting") return;
    if (cloudOnline === true) {
      if (cloudStatus !== "online") {
        set({ cloudStatus: "online", cloudError: null });
      }
      return;
    }
    if (cloudOnline === false && cloudStatus === "online") {
      set({
        cloudStatus: "offline",
        cloudError: "Disconnected from server",
      });
    }
  },

  syncAccountFromCloud: (state) => {
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

// Shared transport reads auth via this port — never imports account-store.
registerCloudAuthProvider(() => {
  const { deviceId, session, authPhase } = useAccountStore.getState();
  const accessToken = session?.accessToken?.trim() ?? "";
  const accountId = session?.accountId?.trim() ?? "";
  if (!accessToken || !accountId) return null;
  return {
    deviceId,
    accessToken,
    accountId,
    authPhase,
  };
});
