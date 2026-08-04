/**
 * L3a ConversationList hydrate.
 *
 * Hub IM mode: daemon list(project) ∥ HubDigestCache (single account hydrate);
 * merge via conversation-list-merge. Never re-fetch Hub per project.
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
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";
import { syncConversationToCloud } from "@/shared/lib/im-cloud-sync";
import { isHubImMode } from "@/shared/lib/hub-timeline";
import { hubDigestCache } from "@/shared/lib/hub-digest-cache";
import { ensureHubDigestHydrated } from "@/shared/lib/hub-digest-ensure";
import { mergeConversationList } from "@/shared/lib/conversation-list-merge";
import { useAccountStore } from "@/store/account-store";

export function createConversationListActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<WorkspaceState, "loadConversations"> {
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
            const { session, authPhase } = useAccountStore.getState();
            const hubMode = isHubImMode({
              authPhase,
              accessToken: session?.accessToken,
            });

            // Daemon project rows + optional Hub digest hydrate (once).
            const daemonPromise = minosQueryClient
              .fetchQuery({
                queryKey: queryKeys.conversations(projectId),
                queryFn: () => daemonApi.listConversations(projectId),
                ...(quiet ? { staleTime: 0 } : {}),
              })
              .catch((err) => {
                // Auth without host: daemon may be down — still show Hub digests.
                if (hubMode) {
                  console.warn(
                    "[conversation-list] daemon list failed; Hub-only merge",
                    err,
                  );
                  return [] as Awaited<
                    ReturnType<typeof daemonApi.listConversations>
                  >;
                }
                throw err;
              });

            const hubPromise = hubMode
              ? ensureHubDigestHydrated()
              : Promise.resolve();

            const [rows] = await Promise.all([daemonPromise, hubPromise]);
            if (isStale()) return;

            const normalized = rows.map((row) =>
              normalizeDaemonConversation(row, projectId),
            );
            const read = { ...get().readMessageCountById };
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

            let list;
            if (hubMode) {
              // P1: Hub digest is unread SSOT. Do not seed rail unread from
              // readMessageCountById (local baseline is daemon-only fallback).
              const daemonUi = normalized
                .filter((row) => Boolean(row.id))
                .map((row) => {
                  const ui = toUiConversation(row, {}, focused, projectId);
                  return { ...ui, unread: undefined };
                });
              // Never attach Hub-only under every project (would duplicate).
              // Hub-only rows live with empty projectId in a single global set.
              list = mergeConversationList({
                daemonRows: daemonUi,
                hubDigests: hubDigestCache.getAll(),
                projectId,
                includeHubOnly: false,
                focusedConversationId: focused,
                unreadSource: "hub",
              });
            } else {
              // Daemon-only / unauthenticated: local baseline unread track.
              list = normalized
                .filter((row) => Boolean(row.id))
                .map((row) => toUiConversation(row, read, focused, projectId));
            }

            // Account-scoped Hub-only rows (no daemon shell): keep once with
            // empty projectId so multi-project rails don't thrash duplicates.
            const hubOnly =
              hubMode && !quiet
                ? mergeConversationList({
                    daemonRows: [],
                    hubDigests: hubDigestCache.getAll().filter((d) => {
                      const allDaemonIds = new Set(
                        get()
                          .conversations.filter((c) => c.projectId)
                          .map((c) => c.id)
                          .concat(list.map((c) => c.id)),
                      );
                      return !allDaemonIds.has(d.conversationId);
                    }),
                    projectId: "",
                    includeHubOnly: true,
                    focusedConversationId: focused,
                    unreadSource: "hub",
                  })
                : [];

            const others = get().conversations.filter(
              (c) => c.projectId !== projectId && Boolean(c.projectId),
            );
            const conversations = [...others, ...list, ...hubOnly];
            set((s) => ({
              conversations,
              // Hub mode: keep map for cold-start daemon-only fallback only;
              // rail unread never reads it while authenticated.
              readMessageCountById: hubMode ? s.readMessageCountById : read,
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

            if (!quiet) {
              for (const row of list) {
                if (!row.id || !row.title?.trim()) continue;
                void syncConversationToCloud({
                  conversationId: row.id,
                  title: row.title,
                  agentRuntimes: row.participatingAgents,
                });
              }
            }
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
