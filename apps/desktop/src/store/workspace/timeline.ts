/**
 * L3a Timeline (messages) hydrate + older pages.
 *
 * Phase 3–4 Linked: Hub chat bubbles are primary SSOT projection; local daemon
 * contributes tool/git/system cards only. loadOlder uses Hub `before_ts_ms`
 * gap API when Hub-first mode. Local-only / unauthenticated still reads fully
 * from daemon.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus, toUiMessage } from "./helpers";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import {
  EMPTY_MESSAGE_HISTORY,
  MESSAGE_PAGE_SIZE,
  firstMessageCreatedAtMs,
  firstMessageSeq,
  mergeMessagesOlder,
  mergeMessagesQuietTail,
  metaAfterMessageTail,
  type MessageHistoryMeta,
  trimMessagesHardMax,
} from "@/shared/lib/message-history";
import { useReactionStore } from "@/features/chat/reaction-store";
import {
  flushImOutbox,
  projectMissingLocalAgentResultsToHub,
} from "@/shared/lib/im-cloud-sync";
import { pullHubConversationMessagePage } from "@/shared/lib/im-cloud-inbound";
import {
  isHubImMode,
  mergeHubAndLocalTimeline,
} from "@/shared/lib/hub-timeline";
import { useAccountStore } from "@/store/account-store";

function hubImEnabled(): boolean {
  const { session, authPhase } = useAccountStore.getState();
  return isHubImMode({
    authPhase,
    accessToken: session?.accessToken,
  });
}

export function createTimelineActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadTimeline"
  | "loadOlderMessages"
> {
  return {
  loadTimeline: async (conversationId, opts) => {
    // Mock: messages already in mockBundle; no daemon RPC.
    if (get().source !== "daemon" || !conversationId) return;
    const quiet = opts?.quiet === true;
    return singleFlightLoad(
      `timeline:${conversationId}:${quiet ? "q" : "h"}`,
      async () => {
        const prev = get().timelineStatusByConversation[conversationId];
        const { next, generation } = bumpStatus(prev, quiet);
        const isStale = () =>
          get().timelineStatusByConversation[conversationId]?.generation !==
          generation;

        set((s) => {
          const dirty = { ...s.timelineDirtyByConversation };
          delete dirty[conversationId];
          // Ensure messages key exists so Hub WS hasWindow during cold pull.
          const hasMessagesKey = Object.prototype.hasOwnProperty.call(
            s.messagesByConversation,
            conversationId,
          );
          return {
            // Mark focused immediately so Hub WS upserts land during the
            // first cold pull (before markConversationRead at end of load).
            focusedConversationId: conversationId,
            messagesByConversation: hasMessagesKey
              ? s.messagesByConversation
              : {
                  ...s.messagesByConversation,
                  [conversationId]: [],
                },
            timelineStatusByConversation: {
              ...s.timelineStatusByConversation,
              [conversationId]: next,
            },
            timelineDirtyByConversation: dirty,
          };
        });

        try {
          const linked = hubImEnabled();

          // Always (re)subscribe when loading a conversation — including quiet
          // refreshes — so a focused timeline never loses live Hub push after
          // reconnect / background loadTimeline(quiet: true).
          if (linked) {
            void import("@/shared/lib/im-hub-bridge").then(
              ({ focusConversationOnHub }) => {
                focusConversationOnHub(conversationId);
              },
            );
          }

          // Hub-first cold/hot hydrate (no daemon append of cloud IM).
          const hubPromise = linked
            ? pullHubConversationMessagePage(conversationId, {
                limit: MESSAGE_PAGE_SIZE,
              })
            : Promise.resolve({
                messages: [] as Awaited<
                  ReturnType<typeof pullHubConversationMessagePage>
                >["messages"],
                nextBeforeTsMs: null as number | null,
                rawCount: 0,
              });

          const messagePage = await daemonApi.listMessages(conversationId, {
            limit: MESSAGE_PAGE_SIZE,
          });
          if (isStale()) return;

          const hubPage = await hubPromise;
          if (isStale()) return;
          const hubRows = hubPage.messages;

          const localUi = messagePage.messages.map(toUiMessage);
          // Durable reactions ride on list_messages; hydrate before paint merge.
          // Phase 5.2: local-only for Linked Hub bubbles — see reaction-store.
          useReactionStore
            .getState()
            .hydrateFromMessages(messagePage.messages);

          set((s) => {
            const prevMessages = s.messagesByConversation[conversationId];
            let merged: typeof localUi;
            if (linked) {
              // Hub SSOT for chat bubbles. Include prev window so optimistic
              // sending/failed + local tool cards survive; strip local chat rows.
              merged = mergeHubAndLocalTimeline({
                hubMessages: hubRows,
                localMessages: [...localUi, ...(prevMessages ?? [])],
              });
            } else {
              // Local-only: union-merge by id with previous window.
              merged = mergeMessagesQuietTail(prevMessages, localUi);
            }
            const trimmed = trimMessagesHardMax(merged);
            merged = trimmed.messages;
            const prevHist =
              s.messageHistoryByConversation[conversationId] ??
              EMPTY_MESSAGE_HISTORY;
            const hubHasOlder =
              linked &&
              (hubPage.nextBeforeTsMs != null ||
                hubPage.rawCount >= MESSAGE_PAGE_SIZE);
            const firstCreated = firstMessageCreatedAtMs(merged);
            const nextHist: MessageHistoryMeta = quiet
              ? {
                  firstLoadedSeq:
                    firstMessageSeq(merged) ?? prevHist.firstLoadedSeq,
                  firstLoadedCreatedAtMs:
                    firstCreated ?? prevHist.firstLoadedCreatedAtMs,
                  hasOlder:
                    prevHist.hasOlder ||
                    messagePage.hasMore ||
                    hubHasOlder ||
                    trimmed.trimmed,
                  loadingOlder: false,
                }
              : {
                  ...metaAfterMessageTail(
                    merged,
                    messagePage.hasMore || hubHasOlder || trimmed.trimmed,
                    firstCreated,
                  ),
                };
            return {
              messagesByConversation: {
                ...s.messagesByConversation,
                [conversationId]: merged,
              },
              messageHistoryByConversation: {
                ...s.messageHistoryByConversation,
                [conversationId]: nextHist,
              },
              timelineStatusByConversation: {
                ...s.timelineStatusByConversation,
                [conversationId]: { phase: "ready", generation },
              },
              error: null,
            };
          });

          // Opening the conversation clears unread message attention.
          if (!quiet) {
            get().markConversationRead(conversationId);
          }

          // User Outbox drain + Host→Hub agent-result uplink for Desktop-native
          // turns (Hub projector only arms client_live Mobile dispatches).
          void flushImOutbox();
          if (linked) {
            void projectMissingLocalAgentResultsToHub(
              conversationId,
              localUi,
              hubRows,
            ).then(async () => {
              if (isStale()) return;
              const hasLocalAgent = localUi.some(
                (m) =>
                  m.role === "agent" || m.id.startsWith("agent-result:"),
              );
              if (!hasLocalAgent) return;
              // Quiet re-pull so Hub echo replaces local-only agent-result
              // (session-keyed merge) after successful uplink.
              const page = await pullHubConversationMessagePage(
                conversationId,
                { limit: MESSAGE_PAGE_SIZE },
              );
              if (isStale()) return;
              set((s) => {
                const prev = s.messagesByConversation[conversationId] ?? [];
                const merged = mergeHubAndLocalTimeline({
                  hubMessages: page.messages,
                  localMessages: [...localUi, ...prev],
                });
                const trimmed = trimMessagesHardMax(merged);
                return {
                  messagesByConversation: {
                    ...s.messagesByConversation,
                    [conversationId]: trimmed.messages,
                  },
                };
              });
            });
          }
        } catch (e) {
          if (isStale()) return;
          const message = e instanceof Error ? e.message : String(e);
          set((s) => ({
            timelineStatusByConversation: {
              ...s.timelineStatusByConversation,
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

  loadOlderMessages: async (conversationId) => {
    if (get().source !== "daemon" || !conversationId) return;
    const hist =
      get().messageHistoryByConversation[conversationId] ??
      EMPTY_MESSAGE_HISTORY;
    if (hist.loadingOlder || !hist.hasOlder) return;

    const linked = hubImEnabled();
    const beforeSeq = hist.firstLoadedSeq;
    const existing = get().messagesByConversation[conversationId] ?? [];
    const beforeTs =
      hist.firstLoadedCreatedAtMs ?? firstMessageCreatedAtMs(existing);

    // Nothing to page with.
    if (!linked && (beforeSeq == null || beforeSeq <= 1)) {
      set((s) => ({
        messageHistoryByConversation: {
          ...s.messageHistoryByConversation,
          [conversationId]: {
            firstLoadedSeq: hist.firstLoadedSeq,
            firstLoadedCreatedAtMs: hist.firstLoadedCreatedAtMs,
            hasOlder: false,
            loadingOlder: false,
          },
        },
      }));
      return;
    }
    if (linked && (beforeTs == null || beforeTs <= 0) && (beforeSeq == null || beforeSeq <= 1)) {
      set((s) => ({
        messageHistoryByConversation: {
          ...s.messageHistoryByConversation,
          [conversationId]: {
            firstLoadedSeq: hist.firstLoadedSeq,
            firstLoadedCreatedAtMs: hist.firstLoadedCreatedAtMs,
            hasOlder: false,
            loadingOlder: false,
          },
        },
      }));
      return;
    }

    set((s) => ({
      messageHistoryByConversation: {
        ...s.messageHistoryByConversation,
        [conversationId]: {
          ...hist,
          loadingOlder: true,
        },
      },
    }));

    try {
      // Local-only path: page daemon. Linked: Hub before_ts_ms + local tool cards.
      const daemonPagePromise =
        beforeSeq != null && beforeSeq > 1
          ? daemonApi.listMessages(conversationId, {
              beforeSeq,
              limit: MESSAGE_PAGE_SIZE,
            })
          : Promise.resolve({ messages: [], hasMore: false });

      const hubPagePromise =
        linked && beforeTs != null && beforeTs > 0
          ? pullHubConversationMessagePage(conversationId, {
              beforeTsMs: beforeTs,
              limit: MESSAGE_PAGE_SIZE,
            })
          : Promise.resolve({
              messages: [] as Awaited<
                ReturnType<typeof pullHubConversationMessagePage>
              >["messages"],
              nextBeforeTsMs: null as number | null,
              rawCount: 0,
            });

      const [page, hubPage] = await Promise.all([
        daemonPagePromise,
        hubPagePromise,
      ]);
      if (get().source !== "daemon") return;
      useReactionStore.getState().hydrateFromMessages(page.messages);
      const olderLocal = page.messages.map(toUiMessage);
      set((s) => {
        const existingWindow = s.messagesByConversation[conversationId] ?? [];
        let older = olderLocal;
        if (linked) {
          older = mergeHubAndLocalTimeline({
            hubMessages: hubPage.messages,
            localMessages: olderLocal,
          });
        }
        const mergedRaw = mergeMessagesOlder(older, existingWindow);
        const trimmed = trimMessagesHardMax(mergedRaw);
        const merged = trimmed.messages;
        const hubHasOlder =
          linked &&
          (hubPage.nextBeforeTsMs != null ||
            hubPage.rawCount >= MESSAGE_PAGE_SIZE);
        return {
          messagesByConversation: {
            ...s.messagesByConversation,
            [conversationId]: merged,
          },
          messageHistoryByConversation: {
            ...s.messageHistoryByConversation,
            [conversationId]: {
              firstLoadedSeq: firstMessageSeq(merged),
              firstLoadedCreatedAtMs: firstMessageCreatedAtMs(merged),
              hasOlder: page.hasMore || hubHasOlder || trimmed.trimmed,
              loadingOlder: false,
            },
          },
        };
      });
    } catch {
      set((s) => {
        const prev =
          s.messageHistoryByConversation[conversationId] ??
          EMPTY_MESSAGE_HISTORY;
        return {
          messageHistoryByConversation: {
            ...s.messageHistoryByConversation,
            [conversationId]: {
              firstLoadedSeq: prev.firstLoadedSeq,
              firstLoadedCreatedAtMs: prev.firstLoadedCreatedAtMs,
              hasOlder: prev.hasOlder,
              loadingOlder: false,
            },
          },
        };
      });
    }
  },

  /**
   * SessionList hydrate — **only** `listProjectSessions(projectId)`.
   * No UI-level list_conversations × list_sessions fan-out.
   */
  };
}
