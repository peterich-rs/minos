/**
 * Agents CLI inventory.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus } from "./helpers";
import { daemonApi } from "@/shared/lib/daemon";
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";


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
      const raw = await minosQueryClient.fetchQuery({
        queryKey: queryKeys.clis,
        queryFn: () => daemonApi.listClis(),
      });
      const clis = raw.map((c) => ({
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
