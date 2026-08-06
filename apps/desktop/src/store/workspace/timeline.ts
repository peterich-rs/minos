/**
 * L3a Timeline (messages) hydrate + older pages.
 *
 * Linked: Hub chat bubbles are primary SSOT projection; local daemon
 * contributes tool/git/system cards only. loadOlder uses Hub `before_seq`
 * when Hub-first mode. Local-only / unauthenticated still reads fully from
 * daemon.
 *
 * focused ≠ hasWindow: loadTimeline is hydrate-only — never writes
 * focusedConversationId and never mark-reads (quiet or full). Focus + mark-read
 * live on open/select (Timeline mount) and debounced focused inbound.
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
          // Ensure messages key exists so Hub WS hasWindow during cold pull.
          // focused ≠ hasWindow: hydrate-only — no focusedConversationId, no
          // mark-read (open path + debounced focused inbound own those).
          const hasMessagesKey = Object.prototype.hasOwnProperty.call(
            s.messagesByConversation,
            conversationId,
          );
          return {
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
          };
        });

        try {
          const linked = hubImEnabled();

          // Subscribe only — loadTimeline is the sole cold-hydrate writer.
          // (focusConversationOnHub must not dual-merge the window.)
          if (linked) {
            void import("@/shared/lib/im-hub-bridge").then(
              ({ ensureConversationSubscribedOnHub }) => {
                ensureConversationSubscribedOnHub(conversationId);
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
                nextBeforeSeq: null as number | null,
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
          // Hub IM: cold-hydrate reactions from mapped Hub rows (id + reactions).
          // Local workbench: daemon list_messages reactions.
          if (linked && hubRows.length > 0) {
            useReactionStore.getState().hydrateFromMessages(hubRows);
          } else {
            useReactionStore
              .getState()
              .hydrateFromMessages(messagePage.messages);
          }

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
              (hubPage.nextBeforeSeq != null ||
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

          // loadTimeline is hydrate-only: never mark-read / never write focus.
          // Focus + Hub mark-read live on open/select (Timeline mount) and
          // debounced inbound while focused (im-hub-bridge).

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
              // (same-id merge) after successful uplink.
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

    // Nothing to page with (daemon and Hub both use message_seq cursors).
    if (beforeSeq == null || beforeSeq <= 1) {
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
      // Local-only: page daemon by seq. Linked: Hub before_seq + local tool cards.
      const daemonPagePromise =
        beforeSeq > 1
          ? daemonApi.listMessages(conversationId, {
              beforeSeq,
              limit: MESSAGE_PAGE_SIZE,
            })
          : Promise.resolve({ messages: [], hasMore: false });

      const hubPagePromise = linked
        ? pullHubConversationMessagePage(conversationId, {
            beforeSeq,
            limit: MESSAGE_PAGE_SIZE,
          })
        : Promise.resolve({
            messages: [] as Awaited<
              ReturnType<typeof pullHubConversationMessagePage>
            >["messages"],
            nextBeforeSeq: null as number | null,
            rawCount: 0,
          });

      const [page, hubPage] = await Promise.all([
        daemonPagePromise,
        hubPagePromise,
      ]);
      if (get().source !== "daemon") return;
      // Linked: Hub page carries viewer-resolved reactions; local-only: daemon.
      useReactionStore
        .getState()
        .hydrateFromMessages(
          linked && hubPage.messages.length > 0
            ? hubPage.messages
            : page.messages,
        );
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
          (hubPage.nextBeforeSeq != null ||
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
