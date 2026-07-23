/**
 * L3a Timeline (messages) hydrate + older pages.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { bumpStatus, toUiMessage } from "./helpers";
import { daemonApi } from "@/shared/lib/daemon";
import { singleFlightLoad } from "@/shared/lib/desktop-inflight";
import {
  EMPTY_MESSAGE_HISTORY,
  MESSAGE_PAGE_SIZE,
  firstMessageSeq,
  mergeMessagesOlder,
  mergeMessagesQuietTail,
  metaAfterMessageTail,
  type MessageHistoryMeta,
  trimMessagesHardMax,
} from "@/shared/lib/message-history";
import { reuseStableById, timelineMessageEqual } from "@/shared/lib/list-identity";
import { useReactionStore } from "@/features/chat/reaction-store";


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
          return {
            timelineStatusByConversation: {
              ...s.timelineStatusByConversation,
              [conversationId]: next,
            },
            timelineDirtyByConversation: dirty,
          };
        });

        try {
          const messagePage = await daemonApi.listMessages(conversationId, {
            limit: MESSAGE_PAGE_SIZE,
          });
          if (isStale()) return;

          const uiMessages = messagePage.messages.map(toUiMessage);
          // Durable reactions ride on list_messages; hydrate before paint merge.
          useReactionStore
            .getState()
            .hydrateFromMessages(messagePage.messages);
          set((s) => {
            const prevMessages = s.messagesByConversation[conversationId];
            // Quiet re-list must keep already-loaded older pages and reuse row
            // identity so TimelineRow/MarkdownText do not remount/reparse.
            let merged = quiet
              ? mergeMessagesQuietTail(prevMessages, uiMessages)
              : reuseStableById(
                  prevMessages,
                  uiMessages,
                  timelineMessageEqual,
                );
            const trimmed = trimMessagesHardMax(merged);
            merged = trimmed.messages;
            const prevHist =
              s.messageHistoryByConversation[conversationId] ??
              EMPTY_MESSAGE_HISTORY;
            // On open: trust daemon hasMore. On quiet: never clear hasOlder if we
            // already know there is history above (tail page alone cannot prove it).
            const nextHist: MessageHistoryMeta = quiet
              ? {
                  firstLoadedSeq:
                    firstMessageSeq(merged) ?? prevHist.firstLoadedSeq,
                  hasOlder:
                    prevHist.hasOlder ||
                    messagePage.hasMore ||
                    trimmed.trimmed,
                  loadingOlder: false,
                }
              : {
                  ...metaAfterMessageTail(merged, messagePage.hasMore),
                  hasOlder: messagePage.hasMore || trimmed.trimmed,
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
    const beforeSeq = hist.firstLoadedSeq;
    if (beforeSeq == null || beforeSeq <= 1) {
      set((s) => ({
        messageHistoryByConversation: {
          ...s.messageHistoryByConversation,
          [conversationId]: {
            firstLoadedSeq: hist.firstLoadedSeq,
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
      const page = await daemonApi.listMessages(conversationId, {
        beforeSeq,
        limit: MESSAGE_PAGE_SIZE,
      });
      if (get().source !== "daemon") return;
      useReactionStore.getState().hydrateFromMessages(page.messages);
      const older = page.messages.map(toUiMessage);
      set((s) => {
        const existing = s.messagesByConversation[conversationId] ?? [];
        const mergedRaw = mergeMessagesOlder(older, existing);
        const trimmed = trimMessagesHardMax(mergedRaw);
        const merged = trimmed.messages;
        return {
          messagesByConversation: {
            ...s.messagesByConversation,
            [conversationId]: merged,
          },
          messageHistoryByConversation: {
            ...s.messageHistoryByConversation,
            [conversationId]: {
              firstLoadedSeq: firstMessageSeq(merged),
              hasOlder: page.hasMore || trimmed.trimmed,
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
