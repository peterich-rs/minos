/**
 * Lifecycle bridge: Account session → Hub realtime → Desktop timeline store.
 *
 * Hub chat messages update `messagesByConversation` directly.
 * Sync Engine: conversation subscribe, resume cursors, SnapshotRequired range rebuild.
 * Cold hydrate is owned by loadTimeline (subscribe-only here on open).
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
import {
  positiveMs,
  railActivityFromTimeline,
  resolveDigestLastActivityMs,
} from "@/shared/lib/rail-activity";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import type { HubChatMessage } from "@/shared/lib/minos-cloud";
import type { TimelineMessage } from "@/shared/lib/mock-data";

let session: HubRealtimeSession | null = null;
let startedForToken: string | null = null;
let lastSyncState: HubRealtimeSyncState = "disconnected";
/** C6.1 lifecycle listeners registered once per process. */
let lifecycleBound = false;

/**
 * Process-local set of message ids already counted toward rail unread.
 * Caps size so T1 conversation frames + T2 account digests for the same
 * message never double-increment unread.
 */
const unreadCountedMessageIds = new Set<string>();
const MAX_UNREAD_COUNTED_IDS = 4000;

function rememberUnreadCounted(messageId: string): boolean {
  const id = messageId.trim();
  if (!id) return false;
  if (unreadCountedMessageIds.has(id)) return false;
  unreadCountedMessageIds.add(id);
  if (unreadCountedMessageIds.size > MAX_UNREAD_COUNTED_IDS) {
    const drop = unreadCountedMessageIds.size - MAX_UNREAD_COUNTED_IDS;
    let i = 0;
    for (const k of unreadCountedMessageIds) {
      unreadCountedMessageIds.delete(k);
      i += 1;
      if (i >= drop) break;
    }
  }
  return true;
}

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
  // Open/focused window: apply body. Background: next open loadTimeline always
  // cold-pulls Hub (no separate dirty flag).
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
  patchRailFromDigest({
    conversationId: message.conversationId,
    messageId: message.messageId,
    preview: opts.isRecall
      ? "Message recalled"
      : message.text?.trim() || null,
    // Never forge client now — 0 means "unknown / omit bump".
    lastAt: positiveMs(message.createdAtMs),
    senderAccountId: message.senderAccountId,
    isRecall: opts.isRecall,
  });
}

/**
 * R3: account-topic thin digest → rail/inbox only (no timeline body).
 * Conversation full frames still call {@link patchRailFromHubMessage}.
 * Unread is messageId-deduped so T1 + T2 never double-count the same bubble.
 *
 * lastMessageAtMs rules:
 * - Append: monotonic max(frame, previous); no Date.now() invent.
 * - Recall: recompute from open timeline after drop; never apply recalled
 *   message createdAtMs (would regress list clock when recalling an old row).
 */
function patchRailFromDigest(input: {
  conversationId: string;
  messageId?: string | null;
  preview: string | null;
  lastAt: number;
  senderAccountId: string;
  isRecall: boolean;
}): void {
  const conversationId = input.conversationId?.trim();
  if (!conversationId) return;
  const ws = useWorkspaceStore.getState();
  const focused = ws.focusedConversationId === conversationId;
  const prevDigest = hubDigestCache.get(conversationId);
  const timeline = Object.prototype.hasOwnProperty.call(
    ws.messagesByConversation,
    conversationId,
  )
    ? ws.messagesByConversation[conversationId]
    : undefined;

  const prevLast = positiveMs(prevDigest?.lastMessageAtMs);
  const resolvedLastAt = resolveDigestLastActivityMs({
    isRecall: input.isRecall,
    incomingLastAtMs: positiveMs(input.lastAt),
    previousLastMessageAtMs: prevLast,
    timeline,
  });

  let preview: string | null;
  if (input.isRecall) {
    const fromWindow = railActivityFromTimeline(timeline);
    preview =
      fromWindow?.preview ??
      input.preview?.trim() ??
      prevDigest?.preview ??
      "Message recalled";
  } else {
    preview = input.preview?.trim() || prevDigest?.preview || null;
  }

  const myAccountId =
    useAccountStore.getState().session?.accountId?.trim() ?? "";
  const isOwn =
    Boolean(myAccountId) && input.senderAccountId === myAccountId;
  let unread = prevDigest?.unreadCount ?? 0;
  // Own multi-end sends must not inflate local unread.
  // messageId dedupe: same bubble on conversation + account topics once.
  if (!focused && !input.isRecall && !isOwn) {
    const mid = input.messageId?.trim() ?? "";
    if (!mid || rememberUnreadCounted(mid)) {
      unread = unread + 1;
    }
  }
  if (focused) {
    unread = 0;
  }
  hubDigestCache.patchOne(conversationId, {
    preview,
    lastMessageAtMs: resolvedLastAt,
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
            updatedAtMs: resolvedLastAt,
            unread: unread > 0 ? unread : undefined,
            messageCount: 0,
            boardColumn: "backlog" as const,
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
    // Recall may lower the clock (latest bubble gone); append is monotonic.
    const railMs = input.isRecall
      ? resolvedLastAt || positiveMs(row.updatedAtMs)
      : Math.max(positiveMs(row.updatedAtMs), resolvedLastAt);
    next[idx] = {
      ...row,
      preview: preview || row.preview,
      updatedAtMs: railMs,
      unread: unread > 0 ? unread : undefined,
    };
    return { conversations: next };
  });
}

function onAccountInboxDigest(digest: {
  conversationId: string;
  messageId: string;
  preview: string;
  atMs: number;
  senderAccountId: string;
  isRecall: boolean;
}): void {
  const conversationId = digest.conversationId?.trim();
  if (!conversationId || !digest.messageId?.trim()) return;
  // Digest is rail-only; timeline body arrives via conversation topic or
  // next open loadTimeline (no dirty-flag scaffolding).
  const focused =
    useWorkspaceStore.getState().focusedConversationId === conversationId;
  patchRailFromDigest({
    conversationId,
    messageId: digest.messageId,
    preview: digest.preview,
    // Account thin digest: at_ms is server activity time (0 = omit bump).
    lastAt: positiveMs(digest.atMs),
    senderAccountId: digest.senderAccountId,
    isRecall: digest.isRecall,
  });
  if (focused) {
    scheduleFocusedMarkRead(conversationId);
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
 * - Multi-page forward-fill with after_seq until empty or cursor stalls.
 * - Latest page calibrates tail; optional multi-page before_seq repair for floor.
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
      let cursor = maxSeq;
      // Cap pages to avoid unbounded work on huge gaps (still multi-page).
      for (let page = 0; page < 20; page += 1) {
        const forward = await pullHubForwardGap(conversationId, cursor, {
          limit: MESSAGE_PAGE_SIZE,
        });
        if (forward.length === 0) break;
        hubChunks.push(...forward);
        const nextMax = lastMessageSeq(forward);
        if (nextMax == null || nextMax <= cursor) break;
        cursor = nextMax;
        if (forward.length < MESSAGE_PAGE_SIZE) break;
      }
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
  // it, page older via before_seq until floor is covered or pages exhaust.
  let latestMin = firstMessageSeq(latestPage.messages);
  if (minSeq != null && minSeq > 1 && latestMin != null && latestMin > minSeq) {
    try {
      for (let page = 0; page < 20; page += 1) {
        if (latestMin == null || latestMin <= minSeq) break;
        const older = await pullHubConversationMessagePage(conversationId, {
          beforeSeq: latestMin,
          limit: MESSAGE_PAGE_SIZE,
        });
        if (older.messages.length === 0) break;
        hubChunks.push(...older.messages);
        const nextMin = firstMessageSeq(older.messages);
        if (nextMin == null || nextMin >= latestMin) break;
        latestMin = nextMin;
        if (older.messages.length < MESSAGE_PAGE_SIZE) break;
      }
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

  // Hydrate reactions from reconciled Hub rows (cold path).
  try {
    const { useReactionStore } = await import(
      "@/features/chat/reaction-store"
    );
    useReactionStore.getState().hydrateFromMessages(hubRows);
  } catch {
    /* reaction store optional in tests */
  }

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
    onAccountInboxDigest: (digest) => {
      onAccountInboxDigest(digest);
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
 * Subscribe conversation durable topic only (no timeline merge).
 * loadTimeline owns cold hydrate to avoid dual writers.
 */
export function ensureConversationSubscribedOnHub(conversationId: string): void {
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
  session?.subscribeConversation(conversationId);
}

/**
 * @deprecated Prefer ensureConversationSubscribedOnHub + loadTimeline.
 * Kept as alias for call sites that only need subscribe (no dual hydrate).
 */
export function focusConversationOnHub(conversationId: string): void {
  ensureConversationSubscribedOnHub(conversationId);
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
