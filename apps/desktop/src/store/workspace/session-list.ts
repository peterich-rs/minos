/**
 * L3b SessionList (project sessions).
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
import { rowsFromEntities } from "@/shared/lib/session-list-projection";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import { transcriptHasPendingApproval } from "@/shared/lib/session-status";
import { hasTranscriptWorkingSet } from "@/shared/lib/transcript-history";
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";

export function createSessionListActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<WorkspaceState, "loadProjectSessions"> {
  return {
    loadProjectSessions: async (projectId, opts) => {
      // Mock: fixtures already injected; no daemon RPC.
      if (get().source !== "daemon" || !projectId) return;
      const quiet = opts?.quiet === true;
      return singleFlightLoad(
        `projectSessions:${projectId}:${quiet ? "q" : "h"}`,
        async () => {
          const prev = get().projectSessionsStatusByProject[projectId];
          const { next, generation } = bumpStatus(prev, quiet);
          const isStale = () =>
            get().projectSessionsStatusByProject[projectId]?.generation !==
            generation;

          set((s) => ({
            projectSessionsStatusByProject: {
              ...s.projectSessionsStatusByProject,
              [projectId]: next,
            },
          }));

          try {
            // Entity-affecting list: always network (no 30s RQ lifecycle lie).
            const daemonSessions = (
              await minosQueryClient.fetchQuery({
                queryKey: queryKeys.projectSessions(projectId),
                queryFn: () => daemonApi.listProjectSessions(projectId),
                staleTime: 0,
              })
            ).map(toUiSession);
            if (isStale()) return;

            set((s) => {
              const sessionsById = { ...s.sessionsById };
              const orderedIds: string[] = [];
              for (const ds of daemonSessions) {
                // Prefer high-confidence transcript signal when working set exists;
                // otherwise preserve Entity.hasPendingApproval (never false-demote).
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
              const sessions = rowsFromEntities(
                sessionsById,
                orderedIds,
              ) as ProjectSession[];
              const committed = commitHydratedSessionEntities(
                s,
                sessionsById,
                orderedIds,
              );
              return {
                ...committed,
                projectSessionsByProject: {
                  ...committed.projectSessionsByProject,
                  [projectId]: sessions,
                },
                projectSessionsStatusByProject: {
                  ...s.projectSessionsStatusByProject,
                  [projectId]: { phase: "ready", generation },
                },
              };
            });
          } catch (e) {
            if (isStale()) return;
            const message = e instanceof Error ? e.message : String(e);
            set((s) => ({
              projectSessionsStatusByProject: {
                ...s.projectSessionsStatusByProject,
                [projectId]: { phase: "error", generation, error: message },
              },
            }));
          }
        },
      );
    },
  };
}
