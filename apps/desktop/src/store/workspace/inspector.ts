/**
 * L3a Inspector (conversation sessions).
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
import {
  mergeRowsIntoProjectSessionList,
  rowsFromEntities,
} from "@/shared/lib/session-list-projection";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";

export function createInspectorActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<WorkspaceState, "loadInspector"> {
  return {
    loadInspector: async (conversationId, opts) => {
      // Mock: sessions already in mockBundle; no daemon RPC.
      if (get().source !== "daemon" || !conversationId) return;
      const quiet = opts?.quiet === true;
      return singleFlightLoad(
        `inspector:${conversationId}:${quiet ? "q" : "h"}`,
        async () => {
          const prev = get().inspectorStatusByConversation[conversationId];
          const { next, generation } = bumpStatus(prev, quiet);
          const isStale = () =>
            get().inspectorStatusByConversation[conversationId]?.generation !==
            generation;

          set((s) => ({
            inspectorStatusByConversation: {
              ...s.inspectorStatusByConversation,
              [conversationId]: next,
            },
          }));

          try {
            // Entity-affecting list: never serve 30s RQ stale as lifecycle truth.
            const sessions = await minosQueryClient.fetchQuery({
              queryKey: queryKeys.inspectorSessions(conversationId),
              queryFn: () => daemonApi.listSessions(conversationId),
              staleTime: 0,
            });
            if (isStale()) return;

            const daemonSessions = sessions.map(toUiSession);

            // Upsert Entity (sample merge) → membership + aggregates from Entity Σ.
            set((s) => {
              const sessionsById = { ...s.sessionsById };
              const orderedIds: string[] = [];
              for (const ds of daemonSessions) {
                const entity = mergeSessionEntity(sessionsById[ds.id], ds, {
                  lifecycleSource: "sample",
                  // omit pendingApproval → keep entity flag
                });
                sessionsById[ds.id] = entity;
                orderedIds.push(ds.id);
              }
              const heldSessions = rowsFromEntities(
                sessionsById,
                orderedIds,
              ) as ProjectSession[];
              const committed = commitHydratedSessionEntities(
                s,
                sessionsById,
                orderedIds,
                { primaryConversationId: conversationId },
              );
              const projectId =
                committed.conversations.find((c) => c.id === conversationId)
                  ?.projectId ??
                s.conversations.find((c) => c.id === conversationId)
                  ?.projectId ??
                "";
              const projectSessionsByProject = projectId
                ? (mergeRowsIntoProjectSessionList(
                    committed.projectSessionsByProject,
                    projectId,
                    heldSessions,
                  ) as Record<string, ProjectSession[]>)
                : committed.projectSessionsByProject;
              return {
                ...committed,
                sessionsByConversation: {
                  ...committed.sessionsByConversation,
                  [conversationId]: heldSessions,
                },
                projectSessionsByProject,
                inspectorStatusByConversation: {
                  ...s.inspectorStatusByConversation,
                  [conversationId]: { phase: "ready", generation },
                },
                error: null,
              };
            });

            // At most one top-level interrupted session auto-continues on open.
            // Skip on quiet poll to avoid double-continue races.
            // Single path: resumeInterruptedSession (Entity SSOT + projection).
            if (!quiet) {
              const continueTarget = daemonSessions
                .filter(
                  (s) =>
                    !s.parentId && s.needsContinue && s.status !== "done",
                )
                .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
              if (continueTarget) {
                await get().resumeInterruptedSession(continueTarget.id);
              }
            }
            if (isStale()) return;

            // Non-quiet: elevate-only transcript peeks for active sessions.
            if (!quiet) {
              const entities = get().sessionsById;
              const active = daemonSessions.filter((s) => {
                if (s.parentId) return false;
                const ent = entities[s.id];
                return (
                  s.status === "running" ||
                  s.status === "suspended" ||
                  ent?.hasPendingApproval === true ||
                  ent?.status === "needs_approval" ||
                  ent?.status === "running"
                );
              });
              await Promise.all(
                active.map((s) =>
                  get()
                    .loadTranscript(s.id, {
                      tailWindow: 120,
                      quiet: true,
                      approvalStatusPolicy: "elevate-only",
                    })
                    .catch(() => {
                      /* ignore tail peek errors */
                    }),
                ),
              );
              if (isStale()) return;

              // Re-project membership + aggregates from Entity after peeks.
              set((s) => {
                const orderedIds = daemonSessions.map((ds) => ds.id);
                const finalSessions = rowsFromEntities(
                  s.sessionsById,
                  orderedIds,
                ) as ProjectSession[];
                const sibling = commitHydratedSessionEntities(
                  s,
                  s.sessionsById,
                  orderedIds,
                  { primaryConversationId: conversationId },
                );
                return {
                  ...sibling,
                  sessionsByConversation: {
                    ...sibling.sessionsByConversation,
                    [conversationId]: finalSessions,
                  },
                };
              });
            }
          } catch (e) {
            if (isStale()) return;
            const message = e instanceof Error ? e.message : String(e);
            set((s) => ({
              inspectorStatusByConversation: {
                ...s.inspectorStatusByConversation,
                [conversationId]: {
                  phase: "error",
                  generation,
                  error: message,
                },
              },
            }));
          }
        },
      );
    },
  };
}
