/**
 * L3a ConversationList hydrate.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  bumpStatus,
  normalizeDaemonConversation,
  patchProjectAggregates,
  toUiConversation,
} from "./helpers";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";


export function createConversationListActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadConversations"
> {
  return {
  loadConversations: async (projectId, opts) => {
    // Mock: fixtures already injected; no daemon RPC.
    if (get().source !== "daemon" || !projectId) return;
    const quiet = opts?.quiet === true;
    return singleFlightLoad(
      `conversations:${projectId}:${quiet ? "q" : "h"}`,
      async () => {
        const prev = get().conversationsStatusByProject[projectId];
        const { next, generation } = bumpStatus(prev, quiet);
        const isStale = () =>
          get().conversationsStatusByProject[projectId]?.generation !==
          generation;

        set((s) => ({
          conversationsStatusByProject: {
            ...s.conversationsStatusByProject,
            [projectId]: next,
          },
        }));

        try {
          const rows = await daemonApi.listConversations(projectId);
          if (isStale()) return;
          const normalized = rows.map((row) =>
            normalizeDaemonConversation(row, projectId),
          );
          const read = { ...get().readMessageCountById };
          // Baseline first sight so the list doesn't scream "unread" for history.
          for (const row of normalized) {
            if (row.id && read[row.id] === undefined) {
              read[row.id] = row.messageCount;
            }
          }
          const focused = get().focusedConversationId;
          if (focused) {
            const focusedRow = normalized.find((r) => r.id === focused);
            if (focusedRow) {
              read[focused] = focusedRow.messageCount;
            }
          }
          // Stamp query projectId so UI filters never drop rows after wire mapping.
          const list = normalized
            .filter((row) => Boolean(row.id))
            .map((row) => toUiConversation(row, read, focused, projectId));
          const others = get().conversations.filter(
            (c) => c.projectId !== projectId,
          );
          const conversations = [...others, ...list];
          set((s) => ({
            conversations,
            readMessageCountById: read,
            conversationsStatusByProject: {
              ...s.conversationsStatusByProject,
              [projectId]: { phase: "ready", generation },
            },
            projects: patchProjectAggregates(
              s.projects,
              projectId,
              conversations,
            ),
          }));
        } catch (e) {
          if (isStale()) return;
          const message = e instanceof Error ? e.message : String(e);
          set((s) => ({
            conversationsStatusByProject: {
              ...s.conversationsStatusByProject,
              [projectId]: { phase: "error", generation, error: message },
            },
          }));
        }
      },
    );
  },

  };
}
