/**
 * Attention detail queue (not sidebar badge).
 */
import type {
  ProjectSession,
  WorkspaceGet,
  WorkspaceSet,
  WorkspaceState,
} from "./types";
import { bumpStatus, toUiSession } from "./helpers";
import { commitHydratedSessionEntities } from "./projection";
import { mergeSessionEntity } from "@/shared/lib/session-entity";
import { rederiveAttentionFromEntities } from "@/shared/lib/session-list-projection";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import { transcriptHasPendingApproval } from "@/shared/lib/session-status";
import { hasTranscriptWorkingSet } from "@/shared/lib/transcript-history";

export function createAttentionActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<WorkspaceState, "loadAttentionSessions"> {
  return {
    loadAttentionSessions: async (opts) => {
      // Mock: attentionSessions seeded in mockBundle.
      if (get().source !== "daemon") return;
      const quiet = opts?.quiet === true;
      return singleFlightLoad(
        `attention:${quiet ? "q" : "h"}`,
        async () => {
          const prev = get().attentionStatus;
          const { next, generation } = bumpStatus(prev, quiet);
          const isStale = () => get().attentionStatus.generation !== generation;

          set({ attentionStatus: next });
          try {
            const projects = get().projects;
            const chunks = await Promise.all(
              projects.map(async (p) => {
                try {
                  return (await daemonApi.listProjectSessions(p.id)).map(
                    toUiSession,
                  );
                } catch {
                  return [] as ProjectSession[];
                }
              }),
            );
            if (isStale()) return;

            set((s) => {
              const sessionsById = { ...s.sessionsById };
              const orderedIds: string[] = [];
              for (const ds of chunks.flat()) {
                let pending: boolean | undefined;
                if (hasTranscriptWorkingSet(s.transcriptsBySession, ds.id)) {
                  pending = transcriptHasPendingApproval(
                    s.transcriptsBySession[ds.id] ?? [],
                  );
                }
                const entity = mergeSessionEntity(sessionsById[ds.id], ds, {
                  pendingApproval: pending,
                  approvalPolicy: "sync",
                  lifecycleSource: "sample",
                });
                sessionsById[ds.id] = entity;
                orderedIds.push(ds.id);
              }
              const committed = commitHydratedSessionEntities(
                {
                  ...s,
                  attentionStatus: { phase: "ready", generation },
                },
                sessionsById,
                orderedIds,
              );
              const attention = rederiveAttentionFromEntities(
                sessionsById,
              ) as ProjectSession[];
              return {
                ...committed,
                attentionSessions: attention,
                attentionStatus: { phase: "ready", generation },
              };
            });
          } catch (e) {
            if (isStale()) return;
            set({
              attentionStatus: {
                phase: "error",
                generation,
                error: e instanceof Error ? e.message : String(e),
              },
            });
          }
        },
      );
    },
  };
}
