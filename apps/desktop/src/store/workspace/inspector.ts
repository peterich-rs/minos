/**
 * L3a Inspector (conversation sessions).
 */
import type { ProjectSession, WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus, patchLocalConversation, toUiSession } from "./helpers";
import { projectHydratedEntities } from "./projection";
import { mergeSessionEntity } from "@/shared/lib/session-entity";
import {
  mergeRowsIntoProjectSessionList,
  rowsFromEntities,
} from "@/shared/lib/session-list-projection";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";


export function createInspectorActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadInspector"
> {
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
          const sessions = await daemonApi.listSessions(conversationId);
          if (isStale()) return;

          const daemonSessions = sessions.map(toUiSession);

          // Upsert Entity first, build membership from Entity projections, then
          // refresh sibling list caches that already contain these session ids.
          set((s) => {
            const sessionsById = { ...s.sessionsById };
            const orderedIds: string[] = [];
            for (const ds of daemonSessions) {
              const entity = mergeSessionEntity(sessionsById[ds.id], ds, {
                // omit pendingApproval → keep entity flag
              });
              sessionsById[ds.id] = entity;
              orderedIds.push(ds.id);
            }
            const heldSessions = rowsFromEntities(
              sessionsById,
              orderedIds,
            ) as ProjectSession[];
            const sibling = projectHydratedEntities(
              s,
              sessionsById,
              orderedIds,
            );
            const runningCount = heldSessions.filter(
              (x) => x.status === "running",
            ).length;
            const approvalCount = heldSessions.filter(
              (x) =>
                x.status === "needs_approval" || x.status === "suspended",
            ).length;
            // Also upsert into project SessionList so Sessions tab (keep-alive)
            // shows every conversation's agents, not only the last full project hydrate.
            const projectId =
              s.conversations.find((c) => c.id === conversationId)?.projectId ??
              "";
            const projectSessionsByProject = projectId
              ? (mergeRowsIntoProjectSessionList(
                  sibling.projectSessionsByProject,
                  projectId,
                  heldSessions,
                ) as Record<string, ProjectSession[]>)
              : sibling.projectSessionsByProject;
            return {
              sessionsById,
              ...sibling,
              sessionsByConversation: {
                ...sibling.sessionsByConversation,
                [conversationId]: heldSessions,
              },
              projectSessionsByProject,
              inspectorStatusByConversation: {
                ...s.inspectorStatusByConversation,
                [conversationId]: { phase: "ready", generation },
              },
              conversations: quiet
                ? patchLocalConversation(s.conversations, conversationId, {
                    runningCount,
                    approvalCount,
                  })
                : s.conversations,
              error: null,
            };
          });

          // At most one top-level interrupted session auto-continues on open.
          // Skip on quiet poll to avoid double-continue races.
          if (!quiet) {
            const continueTarget = daemonSessions
              .filter(
                (s) => !s.parentId && s.needsContinue && s.status !== "done",
              )
              .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
            if (continueTarget) {
              try {
                await daemonApi.resumeSession(continueTarget.id, true);
              } catch {
                /* reattach/continue best-effort; send path can retry */
              }
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
                ent?.status === "needs_approval"
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

            // Re-project Inspector + sibling lists from Entity after peeks.
            set((s) => {
              const orderedIds = daemonSessions.map((ds) => ds.id);
              const finalSessions = rowsFromEntities(
                s.sessionsById,
                orderedIds,
              ) as ProjectSession[];
              const sibling = projectHydratedEntities(
                s,
                s.sessionsById,
                orderedIds,
              );
              const runningCount = finalSessions.filter(
                (x) => x.status === "running",
              ).length;
              const approvalCount = finalSessions.filter(
                (x) =>
                  x.status === "needs_approval" || x.status === "suspended",
              ).length;
              return {
                ...sibling,
                sessionsByConversation: {
                  ...sibling.sessionsByConversation,
                  [conversationId]: finalSessions,
                },
                conversations: patchLocalConversation(
                  s.conversations,
                  conversationId,
                  { runningCount, approvalCount },
                ),
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
