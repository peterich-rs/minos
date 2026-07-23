/**
 * L3b SessionList (project sessions).
 */
import type { ProjectSession, WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus, toUiSession } from "./helpers";
import { projectHydratedEntities } from "./projection";
import { mergeSessionEntity } from "@/shared/lib/session-entity";
import { rowsFromEntities } from "@/shared/lib/session-list-projection";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import { transcriptHasPendingApproval } from "@/shared/lib/session-status";
import { hasTranscriptWorkingSet } from "@/shared/lib/transcript-history";


export function createSessionListActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadProjectSessions"
> {
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
          const daemonSessions = (
            await daemonApi.listProjectSessions(projectId)
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
              });
              sessionsById[ds.id] = entity;
              orderedIds.push(ds.id);
            }
            const sessions = rowsFromEntities(
              sessionsById,
              orderedIds,
            ) as ProjectSession[];
            const sibling = projectHydratedEntities(
              s,
              sessionsById,
              orderedIds,
            );
            return {
              sessionsById,
              ...sibling,
              projectSessionsByProject: {
                ...sibling.projectSessionsByProject,
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
