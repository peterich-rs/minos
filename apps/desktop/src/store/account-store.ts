/**
 * Desktop account session + Host Link state (D01 dual session, D02 link UX).
 *
 * Local coding remains first-class: this store is optional cloud chrome.
 * `hostLink.linked` drives sidebar / Host presence `relayLinked`.
 */

import { create } from "zustand";
import {
  clearStoredHostLink,
  clearStoredSession,
  EMPTY_HOST_LINK,
  ensureDesktopDeviceId,
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
  loginPassword,
  logoutSession,
  registerPassword,
  unlinkHost as cloudUnlinkHost,
  type AuthResponse,
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

export type AuthMode = "login" | "register";

type AccountState = {
  deviceId: string;
  session: MinosSession | null;
  hostLink: HostLinkState;
  authMode: AuthMode;
  busy: boolean;
  error: string | null;
  /** Hydrated once on first import; no async boot required for localStorage. */
  hydrated: boolean;

  setAuthMode: (mode: AuthMode) => void;
  clearError: () => void;

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

export const useAccountStore = create<AccountState>()((set, get) => ({
  deviceId: ensureDesktopDeviceId(),
  session: loadStoredSession(),
  hostLink: loadStoredHostLink(),
  authMode: "login",
  busy: false,
  error: null,
  hydrated: true,

  setAuthMode: (authMode) => set({ authMode, error: null }),
  clearError: () => set({ error: null }),

  isCloudConfigured,
  isSupabaseReady: () => isSupabaseConfigured(),

  signIn: async (email, password) => {
    set({ busy: true, error: null });
    try {
      const deviceId = get().deviceId;
      let auth: AuthResponse;
      if (isSupabaseConfigured()) {
        const supabaseToken = await signInWithSupabasePassword(email, password);
        auth = await exchangeSupabaseSession(deviceId, supabaseToken);
      } else {
        auth = await loginPassword(deviceId, email, password);
      }
      const session = sessionFromAuthResponse(auth);
      saveStoredSession(session);
      set({ session, busy: false, error: null });
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
      const deviceId = get().deviceId;
      let auth: AuthResponse;
      if (isSupabaseConfigured()) {
        const supabaseToken = await signUpWithSupabasePassword(email, password);
        auth = await exchangeSupabaseSession(deviceId, supabaseToken);
      } else {
        auth = await registerPassword(deviceId, email, password);
      }
      const session = sessionFromAuthResponse(auth);
      saveStoredSession(session);
      set({ session, busy: false, error: null });
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
    set({ session: null, busy: false, error: null });
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
