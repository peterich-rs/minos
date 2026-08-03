/**
 * Lifecycle bridge: Account session → Hub realtime → Desktop timeline store.
 *
 * Phase 3–4: Hub chat messages update `messagesByConversation` directly.
 * Sync Engine: conversation subscribe, resume cursors, SnapshotRequired cold rebuild.
 * No daemon_append of cloud IM into Host SQLite.
 */

import { conversationTopic } from "@/shared/lib/hub-cursors";
import {
  HubRealtimeSession,
  type HubRealtimeSyncState,
} from "@/shared/lib/hub-realtime";
import { mapHubChatMessageToTimeline } from "@/shared/lib/im-cloud-inbound";
import {
  isHubImMode,
  mergeHubAndLocalTimeline,
  removeMessageFromTimeline,
  upsertHubMessageIntoTimeline,
} from "@/shared/lib/hub-timeline";
import { startImOutboxWorker } from "@/shared/lib/im-cloud-sync";
import {
  pullHubConversationMessagePage,
  pullHubConversationMessages,
  pullHubForwardGap,
} from "@/shared/lib/im-cloud-inbound";
import {
  EMPTY_MESSAGE_HISTORY,
  MESSAGE_PAGE_SIZE,
  firstMessageCreatedAtMs,
  firstMessageSeq,
  lastMessageSeq,
  mergeMessagesQuietTail,
  trimMessagesHardMax,
} from "@/shared/lib/message-history";
import { hubDigestCache } from "@/shared/lib/hub-digest-cache";
import { ensureHubDigestHydrated } from "@/shared/lib/hub-digest-ensure";
import { formatRelative } from "@/shared/lib/time";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import type { HubChatMessage } from "@/shared/lib/minos-cloud";
import type { TimelineMessage } from "@/shared/lib/mock-data";

let session: HubRealtimeSession | null = null;
let startedForToken: string | null = null;
let lastSyncState: HubRealtimeSyncState = "disconnected";
/** C6.1 lifecycle listeners registered once per process. */
let lifecycleBound = false;

/** Debounced Hub mark-read while a focused timeline receives live messages. */
const MARK_READ_DEBOUNCE_MS = 400;
let markReadTimer: ReturnType<typeof setTimeout> | null = null;
let markReadPendingConversationId: string | null = null;

/**
 * C6.1: visibility / online / focus → pause ping while hidden; force reconnect
 * when shown or network returns and state is not live.
 */
function ensureLifecycleHandlers(): void {
  if (lifecycleBound || typeof window === "undefined") return;
  lifecycleBound = true;

  const maybeForceReconnect = (reason: string) => {
    if (!session) return;
    // Spec C6.1: show/online → if state ≠ live, force reconnect.
    if (session.state === "live") return;
    console.info("[im-hub-bridge] forceReconnect", reason, session.state);
    session.forceReconnect();
  };

  const onVisibility = () => {
    if (!session) return;
    if (typeof document !== "undefined" && document.hidden) {
      // Hide: do not close WS; pause ping to reduce noise.
      session.setPingPaused(true);
      return;
    }
    session.setPingPaused(false);
    if (session.state !== "live") {
      session.forceReconnect();
    }
  };

  const onOnline = () => {
    maybeForceReconnect("window.online");
  };

  const onFocus = () => {
    if (!session) return;
    session.setPingPaused(false);
    if (session.state !== "live") {
      session.forceReconnect();
    }
  };

  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("online", onOnline);
  window.addEventListener("focus", onFocus);

  // Tauri window focus when available (best-effort; no hard dep).
  void import("@tauri-apps/api/window")
    .then(async (mod) => {
      const win = mod.getCurrentWindow?.() ?? null;
      if (!win?.onFocusChanged) return;
      await win.onFocusChanged(({ payload: focused }) => {
        if (focused) {
          onFocus();
        } else {
          session?.setPingPaused(true);
        }
      });
    })
    .catch(() => {
      /* browser / no tauri api */
    });
}

/**
 * Schedule Hub + local mark-read for the focused conversation (C4).
 * Coalesces bursts of inbound messages — same 400ms semantics as Mobile.
 * No-op if focus moved away before the timer fires.
 */
export function scheduleFocusedMarkRead(conversationId: string): void {
  const id = conversationId.trim();
  if (!id) return;
  markReadPendingConversationId = id;
  if (markReadTimer) {
    clearTimeout(markReadTimer);
  }
  markReadTimer = setTimeout(() => {
    markReadTimer = null;
    const pending = markReadPendingConversationId;
    markReadPendingConversationId = null;
    if (!pending) return;
    const focused =
      useWorkspaceStore.getState().focusedConversationId === pending;
    if (!focused) return;
    useWorkspaceStore.getState().markConversationRead(pending);
  }, MARK_READ_DEBOUNCE_MS);
}

/** Test / stop helper: cancel pending debounced mark-read. */
export function cancelPendingFocusedMarkRead(): void {
  if (markReadTimer) {
    clearTimeout(markReadTimer);
    markReadTimer = null;
  }
  markReadPendingConversationId = null;
}

async function onHubChatMessage(message: HubChatMessage): Promise<void> {
  if (message.recalledAtMs) {
    await onHubChatMessageRecalled(message);
    return;
  }
  const ui = await mapHubChatMessageToTimeline(message);
  if (!ui) return;

  const ws = useWorkspaceStore.getState();
  const conversationId = message.conversationId;
  const focused = ws.focusedConversationId === conversationId;
  const hasWindow = Object.prototype.hasOwnProperty.call(
    ws.messagesByConversation,
    conversationId,
  );

  // Always apply into the open/focused conversation window. Previously we only
  // wrote when a messagesByConversation key already existed — a race with the
  // first loadTimeline left Mobile/Hub messages invisible until re-open.
  if (focused || hasWindow) {
    useWorkspaceStore.setState((s) => {
      const prev = s.messagesByConversation[conversationId] ?? [];
      const merged = upsertHubMessageIntoTimeline(prev, ui);
      const trimmed = trimMessagesHardMax(merged);
      const prevHist =
        s.messageHistoryByConversation[conversationId] ?? EMPTY_MESSAGE_HISTORY;
      return {
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: trimmed.messages,
        },
        messageHistoryByConversation: {
          ...s.messageHistoryByConversation,
          [conversationId]: {
            firstLoadedSeq:
              firstMessageSeq(trimmed.messages) ?? prevHist.firstLoadedSeq,
            firstLoadedCreatedAtMs:
              firstMessageCreatedAtMs(trimmed.messages) ??
              prevHist.firstLoadedCreatedAtMs,
            hasOlder: prevHist.hasOlder || trimmed.trimmed,
            loadingOlder: false,
          },
        },
      };
    });
  } else {
    // Background conversation: mark dirty so the next open forces Hub re-list.
    useWorkspaceStore.setState((s) => ({
      timelineDirtyByConversation: {
        ...s.timelineDirtyByConversation,
        [conversationId]: true,
      },
    }));
  }

  // Live patch Hub digest + rail row (no per-project Hub re-query).
  patchRailFromHubMessage(message, { isRecall: false });

  // Focused live inbound: debounce Hub mark-read (not only on open).
  if (focused) {
    scheduleFocusedMarkRead(conversationId);
  }
}

/** Apply account durable / live message into HubDigestCache + workspace rail. */
function patchRailFromHubMessage(
  message: HubChatMessage,
  opts: { isRecall: boolean },
): void {
  const conversationId = message.conversationId;
  if (!conversationId) return;
  const focused =
    useWorkspaceStore.getState().focusedConversationId === conversationId;
  const prevDigest = hubDigestCache.get(conversationId);
  const preview = opts.isRecall
    ? "Message recalled"
    : message.text?.trim() || prevDigest?.preview || null;
  const lastAt = message.createdAtMs || prevDigest?.lastMessageAtMs || Date.now();
  const myAccountId =
    useAccountStore.getState().session?.accountId?.trim() ?? "";
  const isOwn =
    Boolean(myAccountId) && message.senderAccountId === myAccountId;
  let unread = prevDigest?.unreadCount ?? 0;
  // Own multi-end sends must not inflate local unread.
  if (!focused && !opts.isRecall && !isOwn) {
    unread = unread + 1;
  }
  if (focused) {
    unread = 0;
  }
  hubDigestCache.patchOne(conversationId, {
    preview,
    lastMessageAtMs: lastAt,
    unreadCount: unread,
    title: prevDigest?.title,
  });

  useWorkspaceStore.setState((s) => {
    const idx = s.conversations.findIndex((c) => c.id === conversationId);
    if (idx < 0) {
      // Unknown rail row: insert Hub-only shell so multi-end inbox updates.
      return {
        conversations: [
          {
            id: conversationId,
            projectId: "",
            title: prevDigest?.title || "Conversation",
            preview: preview || "No messages yet",
            updatedAt: formatRelative(lastAt),
            updatedAtMs: lastAt,
            unread: unread > 0 ? unread : undefined,
            messageCount: 0,
            boardColumn: "todo" as const,
            agentSessionCount: 0,
            participatingAgents: [],
            runningCount: 0,
            approvalCount: 0,
          },
          ...s.conversations,
        ],
      };
    }
    const next = [...s.conversations];
    const row = next[idx];
    next[idx] = {
      ...row,
      preview: preview || row.preview,
      updatedAt: formatRelative(lastAt),
      updatedAtMs: lastAt,
      unread: unread > 0 ? unread : undefined,
    };
    return { conversations: next };
  });
}

async function onHubChatMessageRecalled(message: HubChatMessage): Promise<void> {
  const conversationId = message.conversationId;
  const messageId = message.messageId;
  if (!conversationId || !messageId) return;

  useWorkspaceStore.setState((s) => {
    if (
      !Object.prototype.hasOwnProperty.call(
        s.messagesByConversation,
        conversationId,
      )
    ) {
      return s;
    }
    const prev = s.messagesByConversation[conversationId];
    const next = removeMessageFromTimeline(prev, messageId);
    if (next === prev) return s;
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [conversationId]: next,
      },
    };
  });
  patchRailFromHubMessage(message, { isRecall: true });
}

function conversationIdFromTopic(topic: string): string | null {
  const prefix = "conversation:";
  if (!topic.startsWith(prefix)) return null;
  const id = topic.slice(prefix.length).trim();
  return id || null;
}

/**
 * SnapshotRequired: range reconcile preferred over clear-only.
 * Connection layer already cleared the topic cursor; rebuild via after_seq /
 * latest-page merge while keeping the existing window as skeleton.
 */
async function onSnapshotRequired(topic: string): Promise<void> {
  const conversationId = conversationIdFromTopic(topic);
  if (!conversationId) {
    // account:* snapshot — invalidate digest cache and cold re-hydrate.
    hubDigestCache.invalidate();
    try {
      await ensureHubDigestHydrated({ force: true });
    } catch (error) {
      console.warn("[im-hub-bridge] account snapshot hydrate failed", error);
    }
    const ws = useWorkspaceStore.getState();
    if (ws.source === "daemon") {
      for (const p of ws.projects) {
        void ws.loadConversations(p.id, { quiet: true });
      }
    }
    return;
  }

  await reconcileConversationFromHub(conversationId);
}

/**
 * Range reconcile for a conversation window after SnapshotRequired.
 * - Keep existing rows as skeleton (no blank flash).
 * - Forward-fill with after_seq = maxLoadedSeq when present.
 * - Latest page merge calibrates tail; merge by id (Hub SSOT for bubbles).
 */
async function reconcileConversationFromHub(
  conversationId: string,
): Promise<void> {
  const prev =
    useWorkspaceStore.getState().messagesByConversation[conversationId] ?? [];
  const maxSeq = lastMessageSeq(prev);
  const minSeq = firstMessageSeq(prev);

  const hubChunks: TimelineMessage[] = [];

  if (maxSeq != null) {
    try {
      const forward = await pullHubForwardGap(conversationId, maxSeq, {
        limit: MESSAGE_PAGE_SIZE,
      });
      hubChunks.push(...forward);
    } catch (error) {
      console.warn(
        "[im-hub-bridge] snapshot forward gap fill failed",
        error,
      );
    }
  }

  // Latest page: always calibrate tail (and cold-open when window empty).
  const latestPage = await pullHubConversationMessagePage(conversationId, {
    limit: MESSAGE_PAGE_SIZE,
  });
  hubChunks.push(...latestPage.messages);

  // When the window has a known min seq and latest page does not cover down to
  // it, one before_seq page repairs the lower edge without clearing UI.
  const latestMin = firstMessageSeq(latestPage.messages);
  if (
    minSeq != null &&
    minSeq > 1 &&
    latestMin != null &&
    latestMin > minSeq
  ) {
    try {
      const older = await pullHubConversationMessagePage(conversationId, {
        beforeSeq: latestMin,
        limit: MESSAGE_PAGE_SIZE,
      });
      hubChunks.push(...older.messages);
    } catch (error) {
      console.warn(
        "[im-hub-bridge] snapshot before_seq repair failed",
        error,
      );
    }
  }

  // Dedupe hub chunk by id (later chunks win).
  const hubById = new Map<string, TimelineMessage>();
  for (const m of hubChunks) {
    hubById.set(m.id, m);
  }
  const hubRows = [...hubById.values()];

  useWorkspaceStore.setState((s) => {
    const localPrev = s.messagesByConversation[conversationId] ?? [];
    const prevHist =
      s.messageHistoryByConversation[conversationId] ?? EMPTY_MESSAGE_HISTORY;
    // Quiet-tail union keeps previously loaded older pages across snapshot.
    const withTail =
      localPrev.length > 0
        ? mergeMessagesQuietTail(localPrev, hubRows)
        : hubRows;
    const merged = mergeHubAndLocalTimeline({
      hubMessages: hubRows,
      localMessages: withTail,
    });
    const trimmed = trimMessagesHardMax(merged);
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [conversationId]: trimmed.messages,
      },
      messageHistoryByConversation: {
        ...s.messageHistoryByConversation,
        [conversationId]: {
          firstLoadedSeq:
            firstMessageSeq(trimmed.messages) ?? prevHist.firstLoadedSeq,
          firstLoadedCreatedAtMs:
            firstMessageCreatedAtMs(trimmed.messages) ??
            prevHist.firstLoadedCreatedAtMs,
          hasOlder:
            prevHist.hasOlder ||
            latestPage.nextBeforeSeq != null ||
            latestPage.rawCount >= MESSAGE_PAGE_SIZE ||
            trimmed.trimmed,
          loadingOlder: false,
        },
      },
    };
  });
}

/** Start or refresh hub WS when Minos account session is available. */
export function ensureImHubBridge(): void {
  const { deviceId, session: account, authPhase } = useAccountStore.getState();
  if (authPhase !== "authenticated" || !account?.accessToken?.trim()) {
    stopImHubBridge();
    return;
  }
  const token = account.accessToken;
  const accountId = account.accountId ?? "";
  if (session && startedForToken === token) {
    session.updateAuth(deviceId, token, accountId);
    // Keep focused conversation subscribed.
    const focused = useWorkspaceStore.getState().focusedConversationId;
    if (focused) {
      session.subscribeConversation(focused);
    }
    return;
  }
  stopImHubBridge();
  session = new HubRealtimeSession({
    onChatMessage: (msg) => {
      void onHubChatMessage(msg);
    },
    onChatMessageRecalled: (msg) => {
      void onHubChatMessageRecalled(msg);
    },
    onMessageReactions: ({ messageId, reactions }) => {
      void import("@/features/chat/reaction-store").then(({ useReactionStore }) => {
        // Durable wire is viewer-neutral; recompute reactedByMe from local account.
        const myAccountId =
          useAccountStore.getState().session?.accountId?.trim() ?? "";
        useReactionStore.getState().applyServerReactions(
          messageId,
          reactions.map((g) => {
            const actors = g.actors.map((a) => ({
              id: a.actorId,
              displayName: a.displayName,
            }));
            const reactedByMe = myAccountId
              ? actors.some((a) => a.id === myAccountId)
              : Boolean(g.reactedByMe);
            return {
              emoji: g.emoji,
              count: g.count,
              reactedByMe,
              actors,
            };
          }),
          { force: false },
        );
      });
    },
    onSnapshotRequired: (topic) => {
      void onSnapshotRequired(topic);
    },
    onConnectionChange: (state) => {
      lastSyncState = state;
      if (state === "live" || state === "syncing") {
        const focused = useWorkspaceStore.getState().focusedConversationId;
        if (focused) {
          session?.subscribeConversation(focused);
        }
      }
    },
  });
  startedForToken = token;
  session.start(deviceId, token, accountId);
  ensureLifecycleHandlers();
  // Drain durable Desktop → Hub user-message Outbox after auth is ready.
  startImOutboxWorker();
}

export function stopImHubBridge(): void {
  session?.stop();
  session = null;
  startedForToken = null;
  lastSyncState = "disconnected";
  cancelPendingFocusedMarkRead();
}

export function getHubRealtimeState(): HubRealtimeSyncState {
  return session?.state ?? lastSyncState;
}

/**
 * Subscribe open conversation topic + cold hydrate (Hub-first).
 * Call on conversation open when account is authenticated.
 */
export function focusConversationOnHub(conversationId: string): void {
  if (!conversationId.trim()) return;
  ensureImHubBridge();
  const { session: account, authPhase } = useAccountStore.getState();
  if (
    !isHubImMode({
      authPhase,
      accessToken: account?.accessToken,
    })
  ) {
    return;
  }
  // Ensure focused id is tracked for reconnect re-subscribe.
  session?.subscribeConversation(conversationId);
  void syncOpenConversationFromHub(conversationId);
}

/**
 * Cold hydrate open conversation from Hub into timeline store (Hub-first).
 * Call on conversation open when account is authenticated.
 */
export async function syncOpenConversationFromHub(
  conversationId: string,
): Promise<void> {
  const { session: account, authPhase } = useAccountStore.getState();
  if (
    !isHubImMode({
      authPhase,
      accessToken: account?.accessToken,
    })
  ) {
    return;
  }

  // Ensure conversation durable topic is subscribed for live append/recall.
  session?.subscribeConversation(conversationId);

  const hubRows = await pullHubConversationMessages(conversationId);
  if (hubRows.length === 0) {
    // Still stamp history meta so loadOlder can try Hub paging.
    useWorkspaceStore.setState((s) => {
      if (
        !Object.prototype.hasOwnProperty.call(
          s.messagesByConversation,
          conversationId,
        )
      ) {
        return s;
      }
      const prevHist =
        s.messageHistoryByConversation[conversationId] ?? EMPTY_MESSAGE_HISTORY;
      return {
        messageHistoryByConversation: {
          ...s.messageHistoryByConversation,
          [conversationId]: {
            ...prevHist,
            firstLoadedCreatedAtMs:
              firstMessageCreatedAtMs(
                s.messagesByConversation[conversationId] ?? [],
              ) ?? prevHist.firstLoadedCreatedAtMs,
          },
        },
      };
    });
    return;
  }

  useWorkspaceStore.setState((s) => {
    const prev = s.messagesByConversation[conversationId] ?? [];
    // Hub SSOT for chat bubbles; strip local user/agent rows that Hub owns.
    const merged = mergeHubAndLocalTimeline({
      hubMessages: hubRows,
      localMessages: prev,
    });
    const trimmed = trimMessagesHardMax(merged);
    const prevHist =
      s.messageHistoryByConversation[conversationId] ?? EMPTY_MESSAGE_HISTORY;
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [conversationId]: trimmed.messages,
      },
      messageHistoryByConversation: {
        ...s.messageHistoryByConversation,
        [conversationId]: {
          firstLoadedSeq:
            firstMessageSeq(trimmed.messages) ?? prevHist.firstLoadedSeq,
          firstLoadedCreatedAtMs:
            firstMessageCreatedAtMs(trimmed.messages) ??
            prevHist.firstLoadedCreatedAtMs,
          hasOlder:
            prevHist.hasOlder ||
            hubRows.length >= MESSAGE_PAGE_SIZE ||
            trimmed.trimmed,
          loadingOlder: false,
        },
      },
    };
  });

  const ws = useWorkspaceStore.getState();
  if (ws.source === "daemon") {
    const conv = ws.conversations.find((c) => c.id === conversationId);
    if (conv?.projectId) {
      void ws.loadConversations(conv.projectId, { quiet: true });
    }
  }
}

/**
 * Phase 5.1: Hub recall for Linked mode.
 * POST Hub recall API then remove from Hub-sourced timeline projection.
 */
export async function recallMessageOnHub(
  conversationId: string,
  messageId: string,
): Promise<void> {
  const { deviceId, session, authPhase } = useAccountStore.getState();
  if (
    !isHubImMode({
      authPhase,
      accessToken: session?.accessToken,
    }) ||
    !session?.accessToken
  ) {
    throw new Error("Hub recall requires authenticated account");
  }
  const { recallHubMessage } = await import("@/shared/lib/minos-cloud");
  const recalled = await recallHubMessage(
    deviceId,
    session.accessToken,
    conversationId,
    messageId,
  );
  await onHubChatMessageRecalled(recalled);
}

/** Topic string helper for tests / diagnostics. */
export function hubConversationTopic(conversationId: string): string {
  return conversationTopic(conversationId);
}
