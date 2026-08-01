/**
 * Desktop account session + Host Link state (D01 dual session, D02 link UX).
 *
 * Product default is **account-first**: a valid Minos session is required
 * before AppShell. Host Link remains a separate second step after sign-in.
 */

import { create } from "zustand";
import {
  clearStoredHostLink,
  clearStoredSession,
  EMPTY_HOST_LINK,
  ensureDesktopDeviceId,
  isAccessTokenFresh,
  loadStoredHostLink,
  loadStoredSession,
  saveStoredHostLink,
  saveStoredSession,
  sessionFromAuthResponse,
  type HostLinkState,
  type MinosSession,
} from "@/shared/lib/account-session";
import {
  backendHttpBase,
  exchangeSupabaseSession,
  linkHost as cloudLinkHost,
  logoutSession,
  refreshSession,
  unlinkHost as cloudUnlinkHost,
} from "@/shared/lib/minos-cloud";
import {
  isSupabaseConfigured,
  signInWithSupabasePassword,
  signOutSupabase,
  signUpWithSupabasePassword,
} from "@/shared/lib/supabase";
import { PROJECT_HOST_THIS_MAC } from "@/shared/lib/host-status";
import { daemonApi } from "@/shared/lib/daemon";
import {
  runHostLinkFlow,
  runHostUnlinkFlow,
} from "@/features/host/lib/host-link-flow";
import type { DesktopAuthPhase } from "@/shared/lib/desktop-root-gate";

export type AuthMode = "login" | "register";

type AccountState = {
  deviceId: string;
  session: MinosSession | null;
  hostLink: HostLinkState;
  authMode: AuthMode;
  /** Root gate phase (hydrate → login | app). */
  authPhase: DesktopAuthPhase;
  busy: boolean;
  error: string | null;
  /** True after first hydrateAuth settles. */
  hydrated: boolean;

  setAuthMode: (mode: AuthMode) => void;
  clearError: () => void;

  /** Cold-start: load local session, refresh if stale, set authPhase. */
  hydrateAuth: () => Promise<void>;

  signIn: (email: string, password: string) => Promise<boolean>;
  signUp: (email: string, password: string) => Promise<boolean>;
  signOut: () => Promise<void>;

  linkThisMac: (hostDisplayName?: string) => Promise<boolean>;
  unlinkThisMac: () => Promise<boolean>;

  /** Whether backend URL is configured for cloud API. */
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

function applyAuthSuccess(
  set: (partial: Partial<AccountState>) => void,
  session: MinosSession,
): void {
  saveStoredSession(session);
  set({
    session,
    authPhase: "authenticated",
    busy: false,
    error: null,
    hydrated: true,
  });
}

export const useAccountStore = create<AccountState>()((set, get) => ({
  deviceId: ensureDesktopDeviceId(),
  session: null,
  hostLink: loadStoredHostLink(),
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
        authPhase: "unauthenticated",
        hydrated: true,
      });
      return;
    }

    if (isAccessTokenFresh(stored)) {
      set({
        session: stored,
        authPhase: "authenticated",
        hydrated: true,
      });
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
      applyAuthSuccess(set, session);
    } catch {
      clearStoredSession();
      set({
        session: null,
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
      applyAuthSuccess(set, sessionFromAuthResponse(auth));
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
      applyAuthSuccess(set, sessionFromAuthResponse(auth));
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
    if (session) {
      try {
        await logoutSession(deviceId, session.accessToken, session.refreshToken);
      } catch {
        // best-effort revoke
      }
    }
    await signOutSupabase();
    clearStoredSession();
    // Host installation token stays on daemon; link flag is independent of
    // human session. Keep hostLink so Linked status survives account re-login.
    set({
      session: null,
      authPhase: "unauthenticated",
      busy: false,
      error: null,
      hydrated: true,
    });
  },

  linkThisMac: async (hostDisplayName) => {
    const session = get().session;
    if (!session) {
      set({ error: "Sign in before linking this Mac" });
      return false;
    }
    set({ busy: true, error: null });
    const deviceId = get().deviceId;
    const displayName = hostDisplayName?.trim() || PROJECT_HOST_THIS_MAC;

    const outcome = await runHostLinkFlow(
      {
        prepareLink: () => daemonApi.hostPrepareLink(),
        signLinkProof: (installationId, nonce) =>
          daemonApi.hostSignLinkProof(installationId, nonce),
        applyLinkToken: (token) => daemonApi.hostApplyLinkToken(token),
        linkHost: (input) =>
          cloudLinkHost(deviceId, session.accessToken, input),
      },
      displayName,
    );

    if (!outcome.linked) {
      set({
        busy: false,
        error: `Link failed (${outcome.stage}): ${outcome.message}`,
      });
      return false;
    }

    const hostLink: HostLinkState = {
      linked: true,
      hostInstallationId: outcome.hostInstallationId,
      hostDisplayName: outcome.hostDisplayName,
      linkedAtMs: outcome.linkedAtMs,
      pairId: outcome.pairId,
    };
    saveStoredHostLink(hostLink);
    set({ hostLink, busy: false, error: null });
    return true;
  },

  unlinkThisMac: async () => {
    const session = get().session;
    const hostId = get().hostLink.hostInstallationId;
    if (!session) {
      set({ error: "Sign in to unlink this Mac" });
      return false;
    }
    if (!hostId) {
      set({ error: "No linked host installation id on this Mac" });
      return false;
    }
    set({ busy: true, error: null });
    const outcome = await runHostUnlinkFlow(
      {
        unlinkHost: (id) =>
          cloudUnlinkHost(get().deviceId, session.accessToken, id),
      },
      hostId,
    );
    if (!outcome.ok) {
      set({ busy: false, error: outcome.message });
      return false;
    }
    clearStoredHostLink();
    set({ hostLink: { ...EMPTY_HOST_LINK }, busy: false, error: null });
    return true;
  },
}));
