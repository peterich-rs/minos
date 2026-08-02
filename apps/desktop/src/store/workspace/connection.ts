/**
 * L1 Connection — bootstrap, project index, livePush arm.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  emptyWorkspace,
  getBootstrapInFlight,
  KNOWN_AGENTS_FALLBACK,
  mockBundle,
  setBootstrapInFlight,
  toUiProject,
} from "./helpers";
import { quietHydrateAllConversationLists } from "./projection";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { startDaemonEventBridge } from "@/shared/lib/daemon-events";
import { resetWorkspaceModuleState } from "./reset-workspace-state";


export function createConnectionActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "bootstrap"
  | "refreshProjects"
  | "refreshDaemonStatus"
  | "clearActionError"
> {
  return {
  /** Refresh IPC + hubOnline without full bootstrap (IM device presence). */
  refreshDaemonStatus: async () => {
    if (!isTauriRuntime()) return;
    if (get().source !== "daemon") return;
    try {
      const connection = await daemonApi.status();
      set({ connection });
    } catch {
      /* keep last snapshot */
    }
  },
  bootstrap: async () => {
    // Single-flight: all concurrent callers (React StrictMode double mount)
    // await the same promise instead of wiping emptyWorkspace twice mid-load.
    const existing = getBootstrapInFlight();
    if (existing) {
      return existing;
    }
    const alreadyReady =
      !get().booting &&
      get().connection?.connected &&
      get().source === "daemon" &&
      get().bootEpoch > 0;
    if (alreadyReady) {
      return;
    }

    setBootstrapInFlight((async () => {
      // Browser-only Vite: mock is intentional for UI work.
      if (!isTauriRuntime()) {
        resetWorkspaceModuleState({
          stopEventBridge: true,
          reactions: "mock-seed",
          clearUiEphemeral: true,
        });
        set({
          ...mockBundle(),
          booting: false,
          bootPhase: "Ready",
          bootProgress: 100,
          bootEpoch: get().bootEpoch + 1,
          connection: null,
          error: null,
          actionError: null,
          loading: false,
          clis: KNOWN_AGENTS_FALLBACK,
          clisStatus: { phase: "ready", generation: 1 },
          attentionStatus: { phase: "ready", generation: 1 },
        });
        return;
      }

      // Hard workspace boundary: drop module singletons before empty caches.
      resetWorkspaceModuleState({
        stopEventBridge: true,
        reactions: "durable-empty",
        clearUiEphemeral: true,
      });
      set({
        booting: true,
        bootPhase: "Connecting to daemon…",
        bootProgress: 12,
        error: null,
        // Never show mock fixtures while booting in Tauri.
        ...emptyWorkspace,
        source: "daemon",
      });

      try {
        set({
          bootPhase: "Starting or discovering daemon…",
          bootProgress: 28,
        });
        const connection = await daemonApi.connect();

        if (!connection.connected) {
          // Caches already wiped at boot start; keep module slate clean.
          resetWorkspaceModuleState({
            stopEventBridge: true,
            reactions: "durable-empty",
            clearUiEphemeral: false,
          });
          set({
            booting: false,
            bootProgress: 100,
            bootPhase: "Daemon unavailable",
            connection,
            error: connection.error,
            source: "daemon",
            ...emptyWorkspace,
            clis: KNOWN_AGENTS_FALLBACK,
            loading: false,
          });
          return;
        }

        set({
          bootPhase: connection.managed
            ? "Managed daemon ready · loading projects…"
            : "Daemon online · loading projects…",
          bootProgress: 55,
          connection,
        });

        const projects = (await daemonApi.listProjects()).map(toUiProject);

        set({ bootPhase: "Loading agents…", bootProgress: 72 });
        let clis = KNOWN_AGENTS_FALLBACK;
        try {
          clis = (await daemonApi.listClis()).map((c) => ({
            agent: c.agent,
            displayName: c.displayName,
            installed: c.installed,
            status: c.status,
            supportsModelSelection: c.supportsModelSelection,
            supportsReasoningEffort: c.supportsReasoningEffort,
          }));
        } catch {
          /* keep fallback */
        }

        // Conversations load lazily when a project view mounts (not all projects).
        // Inflight already cleared at boundary; clear again so ready commit is fresh.
        resetWorkspaceModuleState({
          // Bridge not started yet — no-op stop; re-arm below.
          stopEventBridge: true,
          reactions: "skip",
          clearUiEphemeral: false,
        });
        set({
          booting: false,
          bootPhase: "Ready",
          bootProgress: 100,
          bootEpoch: get().bootEpoch + 1,
          source: "daemon",
          connection,
          loading: false,
          error: null,
          actionError: null,
          focusedConversationId: null,
          ...emptyWorkspace,
          projects,
          clis,
          clisStatus: { phase: "ready", generation: 1 },
        });

        // Arm TUI-parity push subscriptions (ingest / manager / conversation /
        // push-status). Pumps emit daemon://push-status when arming (live=true)
        // or when a current-gen pump ends (live=false → degraded poll).
        try {
          await startDaemonEventBridge({
            onIngest: (ev) => get().applyIngestEvent(ev),
            onManager: (ev) => get().applyManagerEvent(ev),
            onConversation: (ev) => get().applyConversationEvent(ev),
            onPushStatus: (ev) => set({ livePush: Boolean(ev.live) }),
          });
          // Optimistic until first push-status; pumps also emit live=true on arm.
          set({ livePush: true });
        } catch {
          set({ livePush: false });
        }

        // §6.5: quietly hydrate ConversationList for all known projects so badge
        // aggregates (unread + approvalCount from DTO) cover the project index.
        void quietHydrateAllConversationLists(get);
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        resetWorkspaceModuleState({
          stopEventBridge: true,
          reactions: "durable-empty",
          clearUiEphemeral: false,
        });
        set({
          booting: false,
          bootPhase: "Failed",
          bootProgress: 100,
          source: "daemon",
          connection: {
            connected: false,
            endpoint: null,
            error: message,
            source: "error",
            managed: false,
          },
          error: message,
          actionError: null,
          loading: false,
          clis: KNOWN_AGENTS_FALLBACK,
          ...emptyWorkspace,
        });
      }
    })());

    try {
      await getBootstrapInFlight();
    } finally {
      setBootstrapInFlight(null);
    }
  },

  refreshProjects: async () => {
    if (get().source !== "daemon") return;
    try {
      const projects = (await daemonApi.listProjects()).map(toUiProject);
      // Preserve aggregates computed from conversation lists.
      const prev = get().projects;
      const merged = projects.map((p) => {
        const old = prev.find((x) => x.id === p.id);
        return old
          ? {
              ...p,
              needsAttention: old.needsAttention,
              runningAgents: old.runningAgents,
              conversationCount:
                old.conversationCount || p.conversationCount,
              hasUnread: old.hasUnread,
              lastAttentionMs: old.lastAttentionMs,
            }
          : p;
      });
      set({ projects: merged });
      // New projects need quiet ConversationList hydrate for badge coverage.
      void quietHydrateAllConversationLists(get);
    } catch (e) {
      set({
        actionError: e instanceof Error ? e.message : String(e),
      });
    }
  },

  clearActionError: () => set({ actionError: null }),

  };
}
