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
import { pullHubConversationMessages } from "@/shared/lib/im-cloud-inbound";
import {
  EMPTY_MESSAGE_HISTORY,
  MESSAGE_PAGE_SIZE,
  firstMessageCreatedAtMs,
  firstMessageSeq,
  trimMessagesHardMax,
} from "@/shared/lib/message-history";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import type { HubChatMessage } from "@/shared/lib/minos-cloud";

let session: HubRealtimeSession | null = null;
let startedForToken: string | null = null;
let lastSyncState: HubRealtimeSyncState = "disconnected";

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

  // Refresh conversation rail preview/counts when possible.
  if (ws.source === "daemon") {
    const conv = ws.conversations.find((c) => c.id === conversationId);
    if (conv?.projectId) {
      void ws.loadConversations(conv.projectId, { quiet: true });
    } else {
      // Conversation may live under a project not currently loaded in the
      // side list (e.g. opened via hub-only id). Refresh project index +
      // reload conversations for each project so the rail catches up.
      void ws.refreshProjects().then(() => {
        const next = useWorkspaceStore.getState();
        const found = next.conversations.find((c) => c.id === conversationId);
        if (found?.projectId) {
          void next.loadConversations(found.projectId, { quiet: true });
          return;
        }
        for (const p of next.projects) {
          void next.loadConversations(p.id, { quiet: true });
        }
      });
    }
  }
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
}

function conversationIdFromTopic(topic: string): string | null {
  const prefix = "conversation:";
  if (!topic.startsWith(prefix)) return null;
  const id = topic.slice(prefix.length).trim();
  return id || null;
}

/**
 * SnapshotRequired: clear projection window for conversation, cold-pull page,
 * reset is handled by hub-realtime cursor clear.
 */
async function onSnapshotRequired(topic: string): Promise<void> {
  const conversationId = conversationIdFromTopic(topic);
  if (!conversationId) {
    // account:* snapshot — quiet refresh focused conversation list only
    const ws = useWorkspaceStore.getState();
    if (ws.source === "daemon" && ws.focusedConversationId) {
      const conv = ws.conversations.find(
        (c) => c.id === ws.focusedConversationId,
      );
      if (conv?.projectId) {
        void ws.loadConversations(conv.projectId, { quiet: true });
      }
    }
    return;
  }

  // Clear conversation projection then cold-pull snapshot page.
  useWorkspaceStore.setState((s) => ({
    messagesByConversation: {
      ...s.messagesByConversation,
      [conversationId]: [],
    },
    messageHistoryByConversation: {
      ...s.messageHistoryByConversation,
      [conversationId]: { ...EMPTY_MESSAGE_HISTORY },
    },
  }));

  await rebuildConversationFromHub(conversationId);
}

async function rebuildConversationFromHub(
  conversationId: string,
): Promise<void> {
  const hubRows = await pullHubConversationMessages(conversationId);
  useWorkspaceStore.setState((s) => {
    // Hub SSOT for chat bubbles; keep only local tool/git/system + optimistic.
    const prev = s.messagesByConversation[conversationId] ?? [];
    const merged = mergeHubAndLocalTimeline({
      hubMessages: hubRows,
      localMessages: prev,
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
          firstLoadedSeq: firstMessageSeq(trimmed.messages),
          firstLoadedCreatedAtMs: firstMessageCreatedAtMs(trimmed.messages),
          hasOlder:
            hubRows.length >= MESSAGE_PAGE_SIZE || trimmed.trimmed,
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
  if (session && startedForToken === token) {
    session.updateAuth(deviceId, token);
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
  session.start(deviceId, token);
  // Drain durable Desktop → Hub user-message Outbox after auth is ready.
  startImOutboxWorker();
}

export function stopImHubBridge(): void {
  session?.stop();
  session = null;
  startedForToken = null;
  lastSyncState = "disconnected";
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
