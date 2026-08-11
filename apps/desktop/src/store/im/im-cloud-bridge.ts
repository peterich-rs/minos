/**
 * Lifecycle bridge: Account session → Hub realtime → Desktop timeline store.
 *
 * Timeline body writes go through the sole funnel (`timeline-write`), not
 * raw `messagesByConversation` setState. Cold hydrate is owned by
 * loadTimeline (subscribe-only here on open). No daemon_append of cloud IM.
 */

import { conversationTopic } from "@/shared/lib/cloud-cursors";
import {
  CloudRealtimeSession,
  type CloudRealtimeSyncState,
} from "@/shared/lib/cloud-realtime";
import { mapCloudChatMessageToTimeline } from "@/store/im/im-cloud-inbound";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import {
  startImOutboxWorker,
  stopImOutboxWorker,
} from "@/store/im/im-cloud-sync";
import {
  pullCloudConversationMessagePage,
  pullCloudForwardGap,
} from "@/store/im/im-cloud-inbound";
import {
  MESSAGE_PAGE_SIZE,
  firstMessageSeq,
  lastMessageSeq,
} from "@/shared/lib/message-history";
import { cloudDigestCache } from "@/shared/lib/cloud-digest-cache";
import { ensureCloudDigestHydrated } from "@/store/im/cloud-digest-ensure";
import {
  positiveMs,
  railActivityFromTimeline,
  resolveDigestLastActivityMs,
} from "@/shared/lib/rail-activity";
import { getCloudAuth } from "@/shared/lib/cloud-auth";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  applyHubMessage,
  removeRecalledMessage,
  replaceWindowFromHydrate,
} from "@/store/workspace/timeline-write";
import type { CloudChatMessage } from "@/shared/lib/minos-cloud";
import type { TimelineMessage } from "@/shared/domain/collaboration";

let session: CloudRealtimeSession | null = null;
let startedForToken: string | null = null;
let lastSyncState: CloudRealtimeSyncState = "disconnected";
/** Lifecycle listeners registered once per process. */
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
 * visibility / online / focus → pause ping while hidden; force reconnect
 * when shown or network returns and state is not live.
 */
function ensureLifecycleHandlers(): void {
  if (lifecycleBound || typeof window === "undefined") return;
  lifecycleBound = true;

  const maybeForceReconnect = (reason: string) => {
    if (!session) return;
    // show/online → if state ≠ live, force reconnect.
    if (session.state === "live") return;
    console.info("[im-cloud-bridge] forceReconnect", reason, session.state);
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
 * Schedule Hub + local mark-read for the focused conversation.
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

async function onCloudChatMessage(message: CloudChatMessage): Promise<void> {
  if (message.recalledAtMs) {
    await onCloudChatMessageRecalled(message);
    return;
  }
  const ui = await mapCloudChatMessageToTimeline(message);
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
    applyHubMessage(conversationId, ui);
  }

  // Live patch Hub digest + rail row (no per-project Hub re-query).
  patchRailFromCloudMessage(message, { isRecall: false });

  // Focused live inbound: debounce Hub mark-read (not only on open).
  if (focused) {
    scheduleFocusedMarkRead(conversationId);
  }
}

/** Apply account durable / live message into CloudDigestCache + workspace rail. */
function patchRailFromCloudMessage(
  message: CloudChatMessage,
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
 * Conversation full frames still call {@link patchRailFromCloudMessage}.
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
  const prevDigest = cloudDigestCache.get(conversationId);
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

  const myAccountId = getCloudAuth()?.accountId?.trim() ?? "";
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
  cloudDigestCache.patchOne(conversationId, {
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
            // Prefer digest title; empty is fine — list merge keeps daemon title.
            title: prevDigest?.title?.trim() || "",
            preview: preview || "No messages yet",
            updatedAtMs: resolvedLastAt,
            unread: unread > 0 ? unread : undefined,
            messageCount: 0,
            boardColumn: "backlog" as const,
            agentSessionCount: 0,
            participatingBots: [],
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

async function onCloudChatMessageRecalled(message: CloudChatMessage): Promise<void> {
  const conversationId = message.conversationId;
  const messageId = message.messageId;
  if (!conversationId || !messageId) return;

  removeRecalledMessage(conversationId, messageId);
  patchRailFromCloudMessage(message, { isRecall: true });
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
    cloudDigestCache.invalidate();
    try {
      await ensureCloudDigestHydrated({ force: true });
    } catch (error) {
      console.warn("[im-cloud-bridge] account snapshot hydrate failed", error);
    }
    const ws = useWorkspaceStore.getState();
    if (ws.source === "daemon") {
      for (const p of ws.projects) {
        void ws.loadConversations(p.id, { quiet: true });
      }
    }
    return;
  }

  await reconcileConversationFromCloud(conversationId);
}

/**
 * Range reconcile for a conversation window after SnapshotRequired.
 * - Keep existing rows as skeleton (no blank flash).
 * - Multi-page forward-fill with after_seq until empty or cursor stalls.
 * - Latest page calibrates tail; optional multi-page before_seq repair for floor.
 */
async function reconcileConversationFromCloud(
  conversationId: string,
): Promise<void> {
  const prev =
    useWorkspaceStore.getState().messagesByConversation[conversationId] ?? [];
  const maxSeq = lastMessageSeq(prev);
  const minSeq = firstMessageSeq(prev);

  const cloudChunks: TimelineMessage[] = [];

  if (maxSeq != null) {
    try {
      let cursor = maxSeq;
      // Cap pages to avoid unbounded work on huge gaps (still multi-page).
      for (let page = 0; page < 20; page += 1) {
        const forward = await pullCloudForwardGap(conversationId, cursor, {
          limit: MESSAGE_PAGE_SIZE,
        });
        if (forward.length === 0) break;
        cloudChunks.push(...forward);
        const nextMax = lastMessageSeq(forward);
        if (nextMax == null || nextMax <= cursor) break;
        cursor = nextMax;
        if (forward.length < MESSAGE_PAGE_SIZE) break;
      }
    } catch (error) {
      console.warn(
        "[im-cloud-bridge] snapshot forward gap fill failed",
        error,
      );
    }
  }

  // Latest page: always calibrate tail (and cold-open when window empty).
  const latestPage = await pullCloudConversationMessagePage(conversationId, {
    limit: MESSAGE_PAGE_SIZE,
  });
  cloudChunks.push(...latestPage.messages);

  // When the window has a known min seq and latest page does not cover down to
  // it, page older via before_seq until floor is covered or pages exhaust.
  let latestMin = firstMessageSeq(latestPage.messages);
  if (minSeq != null && minSeq > 1 && latestMin != null && latestMin > minSeq) {
    try {
      for (let page = 0; page < 20; page += 1) {
        if (latestMin == null || latestMin <= minSeq) break;
        const older = await pullCloudConversationMessagePage(conversationId, {
          beforeSeq: latestMin,
          limit: MESSAGE_PAGE_SIZE,
        });
        if (older.messages.length === 0) break;
        cloudChunks.push(...older.messages);
        const nextMin = firstMessageSeq(older.messages);
        if (nextMin == null || nextMin >= latestMin) break;
        latestMin = nextMin;
        if (older.messages.length < MESSAGE_PAGE_SIZE) break;
      }
    } catch (error) {
      console.warn(
        "[im-cloud-bridge] snapshot before_seq repair failed",
        error,
      );
    }
  }

  // Dedupe hub chunk by id (later chunks win).
  const cloudById = new Map<string, TimelineMessage>();
  for (const m of cloudChunks) {
    cloudById.set(m.id, m);
  }
  const cloudRows = [...cloudById.values()];

  // Hydrate reactions from reconciled Hub rows (cold path).
  try {
    const { useReactionStore } = await import(
      "@/features/chat/reaction-store"
    );
    useReactionStore.getState().hydrateFromMessages(cloudRows);
  } catch {
    /* reaction store optional in tests */
  }

  // Quiet-tail union keeps previously loaded older pages across snapshot.
  const cloudHasOlder =
    latestPage.nextBeforeSeq != null ||
    latestPage.rawCount >= MESSAGE_PAGE_SIZE;
  replaceWindowFromHydrate(conversationId, {
    mode: "quiet-tail",
    cloudMessages: cloudRows,
    history: {
      hasOlderCloud: cloudHasOlder,
      loadingOlder: false,
    },
  });
}

/** Start or refresh hub WS when Minos account session is available. */
export function ensureImCloudBridge(): void {
  const auth = getCloudAuth();
  if (
    !auth ||
    auth.authPhase !== "authenticated" ||
    !auth.accessToken.trim()
  ) {
    stopImCloudBridge();
    return;
  }
  const deviceId = auth.deviceId;
  const token = auth.accessToken;
  const accountId = auth.accountId;
  if (session && startedForToken === token) {
    session.updateAuth(deviceId, token, accountId);
    // Keep focused conversation subscribed.
    const focused = useWorkspaceStore.getState().focusedConversationId;
    if (focused) {
      session.subscribeConversation(focused);
    }
    return;
  }
  stopImCloudBridge();
  session = new CloudRealtimeSession({
    onChatMessage: (msg) => {
      void onCloudChatMessage(msg);
    },
    onChatMessageRecalled: (msg) => {
      void onCloudChatMessageRecalled(msg);
    },
    onAccountInboxDigest: (digest) => {
      onAccountInboxDigest(digest);
    },
    onMessageReactions: ({ messageId, reactions }) => {
      void import("@/features/chat/reaction-store").then(({ useReactionStore }) => {
        // Durable wire is viewer-neutral; recompute reactedByMe from local account.
        const myAccountId = getCloudAuth()?.accountId?.trim() ?? "";
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
      // Primary product Online = Account IM (`/ws/client`), not Host alone.
      useAccountStore.getState().syncAccountFromCloud(state);
      if (state === "live" || state === "syncing") {
        const focused = useWorkspaceStore.getState().focusedConversationId;
        if (focused) {
          session?.subscribeConversation(focused);
        }
      }
    },
  });
  startedForToken = token;
  // Connecting until first onConnectionChange.
  useAccountStore.getState().syncAccountFromCloud("connecting");
  session.start(deviceId, token, accountId);
  ensureLifecycleHandlers();
  // Drain durable Desktop → Hub user-message Outbox after auth is ready.
  startImOutboxWorker();
}

export function stopImCloudBridge(): void {
  stopImOutboxWorker();
  session?.stop();
  session = null;
  startedForToken = null;
  lastSyncState = "disconnected";
  useAccountStore.getState().syncAccountFromCloud("disconnected");
  cancelPendingFocusedMarkRead();
  // Account leave / stop must not keep unread id set across sessions.
  unreadCountedMessageIds.clear();
}

/** Clear process-local unread dedupe (account leave). */
export function clearImCloudBridgeUnreadDedupe(): void {
  unreadCountedMessageIds.clear();
}

export function getCloudRealtimeState(): CloudRealtimeSyncState {
  return session?.state ?? lastSyncState;
}

/**
 * Prefer Account WS AppendMessage when hub realtime is live.
 * Waits for ChatSendAck/Nack. Returns a result so callers can REST-fallback
 * only on socket/timeout (not on definitive nack, which would double-send).
 */
export async function appendMessageOnCloud(input: {
  clientOperationId: string;
  conversationId: string;
  text: string;
  replyToMessageId?: string | null;
  mentions?: Array<
    | {
        kind: "bot";
        bot_id: string;
        start?: number;
        length?: number;
      }
    | {
        kind: "account";
        account_id: string;
        start?: number;
        length?: number;
      }
  >;
}): Promise<
  | {
      ok: true;
      messageId: string;
      messageSeq: number;
      conversationId: string;
    }
  | { ok: false; reason: "socket" | "timeout" | "nack"; code?: string; message?: string }
> {
  ensureImCloudBridge();
  if (!session) {
    return { ok: false, reason: "socket" };
  }
  const result = await session.sendAppendMessage({
    clientOperationId: input.clientOperationId,
    conversationId: input.conversationId,
    text: input.text,
    replyToMessageId: input.replyToMessageId,
    mentions: input.mentions,
  });
  if (result.ok) {
    return {
      ok: true,
      messageId: result.messageId,
      messageSeq: result.messageSeq,
      conversationId: result.conversationId,
    };
  }
  return {
    ok: false,
    reason: result.reason,
    code: result.code,
    message: result.message,
  };
}

/**
 * @deprecated Fire-and-forget; prefer appendMessageOnCloud for outbox.
 */
export function tryAppendMessageOnCloud(input: {
  clientOperationId: string;
  conversationId: string;
  text: string;
  replyToMessageId?: string | null;
  mentions?: Array<
    | {
        kind: "bot";
        bot_id: string;
        start?: number;
        length?: number;
      }
    | {
        kind: "account";
        account_id: string;
        start?: number;
        length?: number;
      }
  >;
}): boolean {
  ensureImCloudBridge();
  return (
    session?.trySendAppendMessage({
      clientOperationId: input.clientOperationId,
      conversationId: input.conversationId,
      text: input.text,
      replyToMessageId: input.replyToMessageId,
      mentions: input.mentions,
    }) ?? false
  );
}

/**
 * Subscribe conversation durable topic only (no timeline merge).
 * loadTimeline owns cold hydrate to avoid dual writers.
 */
export function ensureConversationSubscribedOnCloud(conversationId: string): void {
  if (!conversationId.trim()) return;
  ensureImCloudBridge();
  const auth = getCloudAuth();
  if (
    !isCloudImMode({
      authPhase: auth?.authPhase,
      accessToken: auth?.accessToken,
    })
  ) {
    return;
  }
  session?.subscribeConversation(conversationId);
}

/**
 * @deprecated Prefer ensureConversationSubscribedOnCloud + loadTimeline.
 * Kept as alias for call sites that only need subscribe (no dual hydrate).
 */
export function focusConversationOnCloud(conversationId: string): void {
  ensureConversationSubscribedOnCloud(conversationId);
}

/**
 * Hub recall for Linked mode.
 * POST Hub recall API then remove from Hub-sourced timeline projection.
 */
export async function recallMessageOnCloud(
  conversationId: string,
  messageId: string,
): Promise<void> {
  const auth = getCloudAuth();
  if (
    !auth ||
    !isCloudImMode({
      authPhase: auth.authPhase,
      accessToken: auth.accessToken,
    }) ||
    !auth.accessToken
  ) {
    throw new Error("Hub recall requires authenticated account");
  }
  const { recallCloudMessage } = await import("@/shared/lib/minos-cloud");
  const recalled = await recallCloudMessage(
    auth.deviceId,
    auth.accessToken,
    conversationId,
    messageId,
  );
  await onCloudChatMessageRecalled(recalled);
}

/** Topic string helper for tests / diagnostics. */
export function cloudConversationTopic(conversationId: string): string {
  return conversationTopic(conversationId);
}
