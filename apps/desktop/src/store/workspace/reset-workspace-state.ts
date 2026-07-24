/**
 * Workspace-boundary reset registry (Buzz `resetCommunityState` counterpart).
 *
 * React remount / Zustand `emptyWorkspace` spreads only clear React state.
 * Module-level Maps, timers, event bridges, and sibling stores survive unless
 * torn down here. Call whenever the data plane is wiped:
 * - Tauri bootstrap start (before emptyWorkspace)
 * - Daemon unavailable / bootstrap failure wipe
 * - Successful ready commit that replaces caches
 * - Browser mock bundle install
 *
 * Project switches stay inside one workspace — they do NOT call this.
 *
 * If you add a new module-level cache scoped to the current daemon session,
 * register its reset below (do not scatter clears inside connection.ts).
 */
import { stopDaemonEventBridge } from "@/shared/lib/daemon-events";
import { clearDesktopInflightState } from "@/shared/lib/desktop-inflight";
import { useReactionStore } from "@/features/chat/reaction-store";
import { useUiStore } from "@/store/ui-store";
import { clearConversationRefreshTimers } from "./empty-workspace";

export type WorkspaceResetReactions = "durable-empty" | "mock-seed" | "skip";

export type ResetWorkspaceModuleStateOptions = {
  /**
   * Stop Tauri `daemon://*` listeners so late frames cannot write into a wiped store.
   * Bridge is re-armed after a successful connect. Default true.
   */
  stopEventBridge?: boolean;
  /**
   * Reaction store:
   * - `durable-empty` — daemon path
   * - `mock-seed` — browser Vite preview
   * - `skip` — leave reactions alone
   * Default `durable-empty`.
   */
  reactions?: WorkspaceResetReactions;
  /**
   * Clear composer drafts / reply chips / selected session (not projectId).
   * Default true on hard wipe.
   */
  clearUiEphemeral?: boolean;
};

/**
 * Tear down every workspace-scoped module singleton.
 * Safe to call multiple times; idempotent for bridge/timers/inflight.
 */
export function resetWorkspaceModuleState(
  options: ResetWorkspaceModuleStateOptions = {},
): void {
  const {
    stopEventBridge = true,
    reactions = "durable-empty",
    clearUiEphemeral = true,
  } = options;

  // 1) Pending timers that close over old conversation ids
  clearConversationRefreshTimers();

  // 2) Resume + single-flight maps (prevents cross-boot resume / stale loads)
  clearDesktopInflightState();

  // 3) Live event bridge (handlers close over get() — restart after connect)
  if (stopEventBridge) {
    stopDaemonEventBridge();
  }

  // 4) Sibling stores
  if (reactions !== "skip") {
    useReactionStore.getState().resetForWorkspaceBoundary(reactions);
  }

  if (clearUiEphemeral) {
    useUiStore.getState().clearWorkspaceEphemeralUi();
  }
}
