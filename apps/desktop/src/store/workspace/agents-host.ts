/**
 * Agents CLI inventory.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus } from "./helpers";
import { daemonApi } from "@/shared/lib/daemon";


export function createAgentsHostActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadClis"
> {
  return {
  loadClis: async (opts) => {
    if (get().source !== "daemon") return;
    const quiet = opts?.quiet === true;
    const prev = get().clisStatus;
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () => get().clisStatus.generation !== generation;
    set({ clisStatus: next });
    try {
      const clis = (await daemonApi.listClis()).map((c) => ({
        agent: c.agent,
        displayName: c.displayName,
        installed: c.installed,
        status: c.status,
        supportsModelSelection: c.supportsModelSelection,
        supportsReasoningEffort: c.supportsReasoningEffort,
      }));
      if (isStale()) return;
      set({ clis, clisStatus: { phase: "ready", generation } });
    } catch (e) {
      if (isStale()) return;
      set({
        clisStatus: {
          phase: "error",
          generation,
          error: e instanceof Error ? e.message : String(e),
        },
      });
    }
  },

  };
}
