/**
 * Desktop Account client → minos-backend formal `/ws/client` realtime.
 *
 * Sync engine:
 *   Disconnected → Connecting → Syncing → Live
 *   per-topic resume_after cursors (localStorage)
 *   Subscribe conversation:{id} when open
 *   SnapshotRequired → clear cursor + cold rebuild callback
 */

import {
  advanceTopicCursor,
  clearTopicCursor,
  conversationTopic,
  loadTopicCursors,
  resumeAfterFromCursors,
  saveTopicCursors,
  type TopicCursorMap,
} from "@/shared/lib/cloud-cursors";
import { conversationSubscriptionLruTouch } from "@/shared/lib/conversation-sub-lru";
import {
  createWsTicket,
  cloudClientWsUrl,
  type HubChatMessage,
} from "@/shared/lib/minos-cloud";

export {
  MAX_OPEN_CONVERSATION_SUBSCRIPTIONS,
  conversationSubscriptionLruTouch,
} from "@/shared/lib/conversation-sub-lru";

export type CloudRealtimeSyncState =
  | "disconnected"
  | "connecting"
  | "syncing"
  | "live"
  | "error";

/** Result of waiting for ChatSendAck/Nack after AppendMessage. */
export type AppendMessageWsResult =
  | {
      ok: true;
      messageId: string;
      messageSeq: number;
      conversationId: string;
    }
  | {
      ok: false;
      reason: "socket" | "timeout" | "nack";
      code?: string;
      message?: string;
    };

type DurableMessagePayload = {
  kind?: string;
  account_id?: string;
  conversation_id?: string;
  message_id?: string;
  at_ms?: number;
  /** R3 account thin digest fields (no nested full message). */
  preview?: string;
  sender_display_name?: string;
  mentioned?: boolean;
  message_seq?: number;
  sender?: {
    kind?: string;
    account_id?: string;
    agent_id?: string;
  };
  message?: {
    message_id: string;
    conversation_id: string;
    text: string;
    created_at_ms: number;
    message_seq?: number;
    sender_type?: string;
    sender: {
      kind?: string;
      account_id?: string;
      minos_id?: string;
      display_name?: string;
      bot_id?: string;
      runtime_agent?: string;
      name?: string | null;
    };
    reply_to?: { message_id: string } | null;
    recalled_at_ms?: number | null;
    mentioned_account_ids?: string[] | null;
    mentioned_agent_ids?: string[] | null;
  };
};

/** Account-topic T2 digest for rail/inbox only (R3). */
export type CloudInboxDigest = {
  conversationId: string;
  messageId: string;
  preview: string;
  atMs: number;
  senderAccountId: string;
  senderDisplayName: string;
  mentioned: boolean;
  messageSeq?: number;
  isRecall: boolean;
};

export type CloudRealtimeHandlers = {
  onChatMessage: (message: HubChatMessage) => void;
  /** Multi-end recall: remove or mark recalled in Hub timeline. */
  onChatMessageRecalled?: (message: HubChatMessage) => void;
  /**
   * Account T2 thin digest — patch rail/inbox only; never full timeline body.
   */
  onAccountInboxDigest?: (digest: CloudInboxDigest) => void;
  /**
   * Conversation reaction aggregate update. Clients apply `reactions` as full
   * replace; `action` is animation-only and must not drive state.
   */
  onMessageReactions?: (input: {
    conversationId: string;
    messageId: string;
    reactions: Array<{
      emoji: string;
      count: number;
      reactedByMe: boolean;
      actors: Array<{
        actorId: string;
        actorKind: string;
        displayName: string;
      }>;
    }>;
  }) => void;
  /** Hub committed AppendMessage (WS write path). */
  onChatSendAck?: (input: {
    clientOperationId: string;
    conversationId: string;
    messageId: string;
    messageSeq: number;
  }) => void;
  /** Hub rejected AppendMessage (validation / membership / …). */
  onChatSendNack?: (input: {
    clientOperationId: string;
    conversationId: string;
    code: string;
    message: string;
  }) => void;
  onConnectionChange?: (state: CloudRealtimeSyncState) => void;
  /**
   * Cursor too old for topic — clear projection for conversation topics and
   * cold-pull snapshot page, then reset cursor.
   */
  onSnapshotRequired?: (topic: string) => void;
  /** Account roster: host linked (T2). */
  onHostLinked?: (input: {
    hostInstallationId: string;
    pairId?: string;
    hostDisplayName?: string;
    atMs?: number;
  }) => void;
  /** Account roster: host unlinked (T2). */
  onHostUnlinked?: (input: { hostInstallationId: string; atMs?: number }) => void;
  /** Gateway subscription cap (default 128). */
  onSubscriptionLimitExceeded?: (input: {
    limit: number;
    current: number;
  }) => void;
};

function mapMessage(
  raw: NonNullable<DurableMessagePayload["message"]>,
): HubChatMessage {
  const s = raw.sender;
  const isBot =
    s.kind === "bot" ||
    raw.sender_type === "agent" ||
    Boolean(s.bot_id && !s.account_id);
  const botId = (s.bot_id ?? s.account_id ?? "").trim();
  const accountId = (s.account_id ?? "").trim();
  return {
    messageId: raw.message_id,
    conversationId: raw.conversation_id,
    text: raw.text,
    createdAtMs: raw.created_at_ms,
    messageSeq: raw.message_seq,
    senderType: isBot ? "agent" : "user",
    // For bots, identity is bot_id (stored in senderAccountId field for less UI churn).
    senderAccountId: isBot ? botId : accountId,
    senderMinosId: isBot
      ? (s.name?.trim() || botId)
      : (s.minos_id ?? "").trim(),
    senderDisplayName: (s.display_name ?? "").trim(),
    runtimeAgent: isBot
      ? (s.runtime_agent?.trim() || undefined)
      : undefined,
    replyToMessageId: raw.reply_to?.message_id ?? null,
    recalledAtMs: raw.recalled_at_ms ?? null,
    mentionedAccountIds: Array.isArray(raw.mentioned_account_ids)
      ? raw.mentioned_account_ids.filter(
          (id): id is string => typeof id === "string" && id.length > 0,
        )
      : undefined,
    mentionedAgentIds: Array.isArray(raw.mentioned_agent_ids)
      ? raw.mentioned_agent_ids.filter(
          (id): id is string => typeof id === "string" && id.length > 0,
        )
      : undefined,
  };
}

/** Conversation topic T1 full message (open chat). */
const CONVERSATION_APPEND_KINDS = new Set([
  "conversation_message_appended",
  "ConversationMessageAppended",
]);

const CONVERSATION_RECALL_KINDS = new Set([
  "conversation_message_recalled",
  "ConversationMessageRecalled",
]);

/** Account topic T2 thin digest (inbox/rail only). */
const ACCOUNT_APPEND_KINDS = new Set([
  "account_conversation_message_appended",
  "AccountConversationMessageAppended",
]);

const ACCOUNT_RECALL_KINDS = new Set([
  "account_conversation_message_recalled",
  "AccountConversationMessageRecalled",
]);

const REACTION_KINDS = new Set([
  "conversation_message_reaction_updated",
  "ConversationMessageReactionUpdated",
]);

function mapAccountDigest(
  payload: DurableMessagePayload,
  isRecall: boolean,
): CloudInboxDigest | null {
  const conversationId = payload.conversation_id?.trim();
  const messageId = payload.message_id?.trim();
  if (!conversationId || !messageId) return null;
  const sender = payload.sender;
  const senderAccountId =
    sender?.account_id?.trim() || sender?.agent_id?.trim() || "";
  const preview = isRecall
    ? (payload.preview?.trim() || "Message recalled")
    : (payload.preview?.trim() ?? "");
  // 0 = omit activity bump (never invent client Date.now()).
  const atMs =
    typeof payload.at_ms === "number" &&
    Number.isFinite(payload.at_ms) &&
    payload.at_ms > 0
      ? payload.at_ms
      : 0;
  return {
    conversationId,
    messageId,
    preview,
    atMs,
    senderAccountId,
    senderDisplayName: payload.sender_display_name?.trim() || "",
    mentioned: Boolean(payload.mentioned),
    messageSeq:
      typeof payload.message_seq === "number" ? payload.message_seq : undefined,
    isRecall,
  };
}

export class CloudRealtimeSession {
  private ws: WebSocket | null = null;
  private stopped = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempt = 0;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  /** When true, skip starting ping (document hidden); resume on show. */
  private pingPaused = false;
  private readonly handlers: CloudRealtimeHandlers;
  private auth: {
    deviceId: string;
    accessToken: string;
    accountId: string;
  } | null = null;
  private syncState: CloudRealtimeSyncState = "disconnected";
  /**
   * Desired conversation topics (open windows); re-subscribed on reconnect.
   * Insertion/access order is LRU: first key = oldest for eviction (R4).
   */
  private conversationIds = new Map<string, true>();
  private cursors: TopicCursorMap = loadTopicCursors();
  /** Topics we still expect SubscribeAck for after a Subscribe batch. */
  private pendingSubscribeTopics = new Set<string>();
  /**
   * Waiters for ChatSendAck/Nack keyed by client_operation_id.
   * Outbox must not mark success on mere WS send.
   */
  private appendWaiters = new Map<
    string,
    {
      resolve: (result: AppendMessageWsResult) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();

  constructor(handlers: CloudRealtimeHandlers) {
    this.handlers = handlers;
  }

  get state(): CloudRealtimeSyncState {
    return this.syncState;
  }

  /**
   * Account → Hub collaboration write when `/ws/client` is live.
   * Resolves only after ChatSendAck/Nack (or timeout). Caller should REST
   * fallback on `{ok:false, reason:"socket"|"timeout"}` — not on nack alone
   * when the op was definitively rejected (avoids double-send).
   */
  sendAppendMessage(
    input: {
      clientOperationId: string;
      conversationId: string;
      text: string;
      replyToMessageId?: string | null;
      /** Structured SSOT mentions (bot/account). Omitted when empty. */
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
    },
    opts?: { timeoutMs?: number },
  ): Promise<AppendMessageWsResult> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      return Promise.resolve({ ok: false, reason: "socket" });
    }
    if (this.syncState !== "live" && this.syncState !== "syncing") {
      return Promise.resolve({ ok: false, reason: "socket" });
    }
    const client_operation_id = input.clientOperationId?.trim();
    const conversation_id = input.conversationId?.trim();
    if (!client_operation_id || !conversation_id) {
      return Promise.resolve({ ok: false, reason: "socket" });
    }
    // Already waiting on this op — share the same promise outcome via a new waiter
    // only if none exists; otherwise treat as socket miss so REST can confirm.
    if (this.appendWaiters.has(client_operation_id)) {
      return Promise.resolve({ ok: false, reason: "socket" });
    }
    const frame: Record<string, unknown> = {
      type: "append_message",
      client_operation_id,
      conversation_id,
      text: input.text ?? "",
    };
    const reply = input.replyToMessageId?.trim();
    if (reply) {
      frame.reply_to_message_id = reply;
    }
    if (input.mentions && input.mentions.length > 0) {
      frame.mentions = input.mentions;
    }
    const timeoutMs = opts?.timeoutMs ?? 8_000;
    return new Promise<AppendMessageWsResult>((resolve) => {
      const timer = setTimeout(() => {
        this.appendWaiters.delete(client_operation_id);
        resolve({ ok: false, reason: "timeout" });
      }, timeoutMs);
      this.appendWaiters.set(client_operation_id, { resolve, timer });
      try {
        this.ws!.send(JSON.stringify(frame));
      } catch {
        clearTimeout(timer);
        this.appendWaiters.delete(client_operation_id);
        resolve({ ok: false, reason: "socket" });
      }
    });
  }

  /**
   * Fire-and-forget helper kept for non-outbox callers. Prefer sendAppendMessage
   * when success must wait for ChatSendAck.
   */
  trySendAppendMessage(input: {
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
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return false;
    if (this.syncState !== "live" && this.syncState !== "syncing") return false;
    const client_operation_id = input.clientOperationId?.trim();
    const conversation_id = input.conversationId?.trim();
    if (!client_operation_id || !conversation_id) return false;
    const frame: Record<string, unknown> = {
      type: "append_message",
      client_operation_id,
      conversation_id,
      text: input.text ?? "",
    };
    const reply = input.replyToMessageId?.trim();
    if (reply) {
      frame.reply_to_message_id = reply;
    }
    if (input.mentions && input.mentions.length > 0) {
      frame.mentions = input.mentions;
    }
    this.ws.send(JSON.stringify(frame));
    return true;
  }

  private settleAppendWaiter(
    clientOperationId: string,
    result: AppendMessageWsResult,
  ): void {
    const waiter = this.appendWaiters.get(clientOperationId);
    if (!waiter) return;
    clearTimeout(waiter.timer);
    this.appendWaiters.delete(clientOperationId);
    waiter.resolve(result);
  }

  start(deviceId: string, accessToken: string, accountId?: string): void {
    this.stopped = false;
    this.auth = {
      deviceId,
      accessToken,
      accountId: accountId?.trim() ?? "",
    };
    this.cursors = loadTopicCursors();
    void this.connect();
  }

  updateAuth(deviceId: string, accessToken: string, accountId?: string): void {
    this.auth = {
      deviceId,
      accessToken,
      accountId: accountId?.trim() ?? this.auth?.accountId ?? "",
    };
  }

  /**
   * C6.1: Force an immediate reconnect (sleep/wake, online, focus restore).
   * Resets backoff, closes the current socket, and connects now.
   */
  forceReconnect(): void {
    if (this.stopped || !this.auth) return;
    this.clearTimers();
    this.reconnectAttempt = 0;
    this.pendingSubscribeTopics.clear();
    if (this.ws) {
      try {
        // Detach onclose so we do not schedule the backoff path.
        this.ws.onclose = null;
        this.ws.onerror = null;
        this.ws.onmessage = null;
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    void this.connect();
  }

  /**
   * Pause keepalive pings while the document is hidden (do **not** close WS).
   * TCP will naturally timeout if the network is gone; on show we reconnect.
   */
  setPingPaused(paused: boolean): void {
    this.pingPaused = paused;
    if (paused) {
      if (this.pingTimer) {
        clearInterval(this.pingTimer);
        this.pingTimer = null;
      }
      return;
    }
    // Resume ping if socket is open.
    this.ensurePingTimer();
  }

  stop(): void {
    this.stopped = true;
    this.clearTimers();
    this.conversationIds.clear();
    this.pendingSubscribeTopics.clear();
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    this.setState("disconnected");
  }

  private ensurePingTimer(): void {
    if (this.pingPaused || this.pingTimer) return;
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    this.pingTimer = setInterval(() => {
      if (this.pingPaused) return;
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(
          JSON.stringify({
            type: "ping",
            ts: Date.now(),
          }),
        );
      }
    }, 25_000);
  }

  /**
   * Open/focus a conversation: ensure `conversation:{id}` is subscribed with
   * resume_after when Live/Syncing. Enforces LRU cap
   * [`MAX_OPEN_CONVERSATION_SUBSCRIPTIONS`] (R4).
   */
  subscribeConversation(conversationId: string): void {
    const id = conversationId?.trim();
    if (!id) return;
    const ordered = [...this.conversationIds.keys()];
    const { next, evicted } = conversationSubscriptionLruTouch(ordered, id);
    for (const old of evicted) {
      this.unsubscribeConversation(old);
    }
    this.conversationIds.clear();
    for (const cid of next) {
      this.conversationIds.set(cid, true);
    }
    this.sendSubscribe([conversationTopic(id)]);
  }

  /** Leave conversation topic when window closed or LRU-evicted. */
  unsubscribeConversation(conversationId: string): void {
    const id = conversationId?.trim();
    if (!id) return;
    if (!this.conversationIds.delete(id)) return;
    const topic = conversationTopic(id);
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(
        JSON.stringify({
          type: "unsubscribe",
          topics: [topic],
        }),
      );
    }
  }

  /** Test/introspection: current conversation subscription count. */
  get openConversationSubscriptionCount(): number {
    return this.conversationIds.size;
  }

  private setState(state: CloudRealtimeSyncState): void {
    if (this.syncState === state) return;
    this.syncState = state;
    this.handlers.onConnectionChange?.(state);
  }

  private clearTimers(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.stopped || !this.auth) return;
    this.clearTimers();
    const delay = Math.min(30_000, 1000 * 2 ** Math.min(this.reconnectAttempt, 5));
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => {
      void this.connect();
    }, delay);
  }

  private persistCursors(): void {
    saveTopicCursors(this.cursors);
  }

  private noteTopicSeq(topic: string | undefined, topicSeq: number | undefined): void {
    if (!topic || topicSeq == null || !Number.isFinite(topicSeq)) return;
    const next = advanceTopicCursor(this.cursors, topic, topicSeq);
    if (next !== this.cursors) {
      this.cursors = next;
      this.persistCursors();
    }
  }

  private desiredTopics(): string[] {
    const topics: string[] = [];
    const accountId = this.auth?.accountId?.trim();
    if (accountId) {
      // Account topic: Hello is register-only; client must Subscribe with resume.
      topics.push(`account:${accountId}`);
    }
    for (const id of this.conversationIds.keys()) {
      topics.push(conversationTopic(id));
    }
    return topics;
  }

  private sendSubscribe(topics: string[]): void {
    if (topics.length === 0) return;
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const unique = [...new Set(topics)];
    for (const t of unique) {
      this.pendingSubscribeTopics.add(t);
    }
    if (this.syncState === "live") {
      this.setState("syncing");
    }
    const resume_after = resumeAfterFromCursors(this.cursors, unique);
    this.ws.send(
      JSON.stringify({
        type: "subscribe",
        topics: unique,
        ...(resume_after ? { resume_after } : {}),
      }),
    );
  }

  private async connect(): Promise<void> {
    if (this.stopped || !this.auth) return;
    this.setState("connecting");
    try {
      const ticket = await createWsTicket(
        this.auth.deviceId,
        this.auth.accessToken,
      );
      if (this.stopped) return;
      const url = cloudClientWsUrl(ticket.gatewayUrl, ticket.ticket);
      const ws = new WebSocket(url);
      this.ws = ws;

      ws.onopen = () => {
        this.reconnectAttempt = 0;
        // Wait for Hello before Subscribe; account is auto-subscribed by gateway.
        this.ensurePingTimer();
      };

      ws.onmessage = (ev) => {
        this.handleRaw(String(ev.data ?? ""));
      };

      ws.onerror = () => {
        this.setState("error");
      };

      ws.onclose = () => {
        this.ws = null;
        this.clearTimers();
        this.pendingSubscribeTopics.clear();
        this.setState("disconnected");
        this.scheduleReconnect();
      };
    } catch (error) {
      console.warn("[cloud-realtime] connect failed", error);
      this.setState("error");
      this.scheduleReconnect();
    }
  }

  private handleRaw(raw: string): void {
    let frame: {
      type?: string;
      kind?: string;
      topic?: string;
      topic_seq?: number;
      payload?: DurableMessagePayload | Record<string, unknown>;
      topics?: string[];
      last_known_seq?: number;
      retention_floor_seq?: number;
      ts?: number;
      server_time_ms?: number;
      conn_id?: string;
      heartbeat_interval_ms?: number;
    };
    try {
      frame = JSON.parse(raw) as typeof frame;
    } catch {
      return;
    }

    const type = frame.type;

    if (type === "hello") {
      this.setState("syncing");
      // Register-only Hello may include a default-topic SubscribeAck next;
      // always client-Subscribe account + open conversations with resume_after.
      const topics = this.desiredTopics();
      if (topics.length > 0) {
        this.sendSubscribe(topics);
      } else {
        this.setState("live");
      }
      return;
    }

    if (type === "subscribe_ack") {
      for (const t of frame.topics ?? []) {
        this.pendingSubscribeTopics.delete(t);
      }
      // Gateway may emit default-topic ack before our Subscribe; ignore extras.
      if (this.pendingSubscribeTopics.size === 0) {
        this.setState("live");
      }
      return;
    }

    if (type === "snapshot_required") {
      const topic = frame.topic ?? "";
      if (topic) {
        this.cursors = clearTopicCursor(this.cursors, topic);
        this.persistCursors();
        this.handlers.onSnapshotRequired?.(topic);
      }
      return;
    }

    if (type === "subscription_denied") {
      console.warn("[cloud-realtime] subscription denied", frame);
      return;
    }

    if (
      type === "subscription_limit_exceeded" ||
      type === "SubscriptionLimitExceeded"
    ) {
      const limit = Number(
        (frame as { limit?: number }).limit ??
          (frame.payload as { limit?: number } | undefined)?.limit ??
          0,
      );
      const current = Number(
        (frame as { current?: number }).current ??
          (frame.payload as { current?: number } | undefined)?.current ??
          0,
      );
      console.warn("[cloud-realtime] subscription limit exceeded", {
        limit,
        current,
      });
      this.handlers.onSubscriptionLimitExceeded?.({ limit, current });
      return;
    }

    if (type === "pong") {
      return;
    }

    if (type === "chat_send_ack" || type === "ChatSendAck") {
      const clientOperationId = String(
        (frame as { client_operation_id?: string }).client_operation_id ?? "",
      ).trim();
      const conversationId = String(
        (frame as { conversation_id?: string }).conversation_id ?? "",
      ).trim();
      const messageId = String(
        (frame as { message_id?: string }).message_id ?? "",
      ).trim();
      const messageSeq = Number(
        (frame as { message_seq?: number }).message_seq ?? 0,
      );
      if (clientOperationId) {
        this.settleAppendWaiter(clientOperationId, {
          ok: true,
          messageId,
          messageSeq,
          conversationId,
        });
        this.handlers.onChatSendAck?.({
          clientOperationId,
          conversationId,
          messageId,
          messageSeq,
        });
      }
      return;
    }

    if (type === "chat_send_nack" || type === "ChatSendNack") {
      const clientOperationId = String(
        (frame as { client_operation_id?: string }).client_operation_id ?? "",
      ).trim();
      const conversationId = String(
        (frame as { conversation_id?: string }).conversation_id ?? "",
      ).trim();
      const code = String((frame as { code?: string }).code ?? "nack");
      const message = String((frame as { message?: string }).message ?? "");
      if (clientOperationId) {
        this.settleAppendWaiter(clientOperationId, {
          ok: false,
          reason: "nack",
          code,
          message,
        });
        this.handlers.onChatSendNack?.({
          clientOperationId,
          conversationId,
          code,
          message,
        });
      }
      return;
    }

    if (type === "durable_event" || type === "DurableEvent") {
      // Advance resume cursor only after a successful apply so a drop
      // (missing payload / map failure) can be replayed on reconnect.
      const applied = this.handleDurable(
        frame.kind,
        frame.payload as DurableMessagePayload | undefined,
      );
      if (applied) {
        this.noteTopicSeq(frame.topic, frame.topic_seq);
      }
      return;
    }

    // Some serializers nest kind on payload only — still advance cursor on apply.
    if (frame.payload && typeof frame.payload === "object" && "kind" in frame.payload) {
      const p = frame.payload as DurableMessagePayload;
      const applied = this.handleDurable(p.kind, p);
      if (applied) {
        this.noteTopicSeq(frame.topic, frame.topic_seq);
      }
    }
  }

  /**
   * @returns true when the event was handled (or intentionally ignored as
   * non-chat durable). false = apply failed; caller must not advance cursor.
   */
  private handleDurable(
    kind: string | undefined,
    payload: DurableMessagePayload | undefined,
  ): boolean {
    if (!payload) return false;
    const eventKind = kind ?? payload.kind;
    if (!eventKind) return false;

    // R3 account thin digest → rail/inbox only (never full timeline body).
    if (ACCOUNT_APPEND_KINDS.has(eventKind)) {
      const digest = mapAccountDigest(payload, false);
      if (!digest) return false;
      this.handlers.onAccountInboxDigest?.(digest);
      return true;
    }
    if (ACCOUNT_RECALL_KINDS.has(eventKind)) {
      const digest = mapAccountDigest(payload, true);
      if (!digest) return false;
      this.handlers.onAccountInboxDigest?.(digest);
      return true;
    }

    // Conversation T1 full frames → open-chat timeline.
    if (CONVERSATION_APPEND_KINDS.has(eventKind)) {
      if (!payload.message) return false;
      try {
        const msg = mapMessage(payload.message);
        if (msg.recalledAtMs) {
          this.handlers.onChatMessageRecalled?.(msg);
        } else {
          this.handlers.onChatMessage(msg);
        }
        return true;
      } catch (error) {
        console.warn("[cloud-realtime] failed to map chat message", error);
        return false;
      }
    }

    if (CONVERSATION_RECALL_KINDS.has(eventKind)) {
      try {
        if (payload.message) {
          this.handlers.onChatMessageRecalled?.(mapMessage(payload.message));
          return true;
        }
        if (payload.message_id && payload.conversation_id) {
          // Minimal recall payload without full message body.
          // at_ms is recall event time; 0 when absent (rail keeps prev clock).
          const atMs =
            typeof payload.at_ms === "number" &&
            Number.isFinite(payload.at_ms) &&
            payload.at_ms > 0
              ? payload.at_ms
              : 0;
          this.handlers.onChatMessageRecalled?.({
            messageId: payload.message_id,
            conversationId: payload.conversation_id,
            text: "",
            createdAtMs: atMs,
            senderType: "user",
            senderAccountId: "",
            senderMinosId: "",
            senderDisplayName: "",
            replyToMessageId: null,
            recalledAtMs: atMs > 0 ? atMs : null,
          });
          return true;
        }
        return false;
      } catch (error) {
        console.warn("[cloud-realtime] failed to map recall", error);
        return false;
      }
    }

    if (REACTION_KINDS.has(eventKind)) {
      const p = payload as DurableMessagePayload & {
        reactions?: Array<{
          emoji: string;
          count: number;
          reacted_by_me?: boolean;
          reactedByMe?: boolean;
          actors?: Array<{
            actor_id?: string;
            actorId?: string;
            actor_kind?: string;
            actorKind?: string;
            display_name?: string;
            displayName?: string;
          }>;
        }>;
      };
      if (!p.message_id || !p.conversation_id || !p.reactions) {
        return false;
      }
      this.handlers.onMessageReactions?.({
        conversationId: p.conversation_id,
        messageId: p.message_id,
        reactions: p.reactions.map((g) => ({
          emoji: g.emoji,
          count: g.count,
          reactedByMe: Boolean(g.reacted_by_me ?? g.reactedByMe),
          actors: (g.actors ?? []).map((a) => ({
            actorId: a.actor_id ?? a.actorId ?? "",
            actorKind: a.actor_kind ?? a.actorKind ?? "user",
            displayName: a.display_name ?? a.displayName ?? "",
          })),
        })),
      });
      return true;
    }

    if (
      eventKind === "host_linked" ||
      eventKind === "HostLinked"
    ) {
      const p = payload as DurableMessagePayload & {
        host_installation_id?: string;
        pair_id?: string;
        host_display_name?: string;
      };
      const hostId = p.host_installation_id?.trim();
      if (!hostId) return false;
      this.handlers.onHostLinked?.({
        hostInstallationId: hostId,
        pairId: p.pair_id,
        hostDisplayName: p.host_display_name,
        atMs: p.at_ms,
      });
      return true;
    }

    if (
      eventKind === "host_unlinked" ||
      eventKind === "HostUnlinked"
    ) {
      const p = payload as DurableMessagePayload & {
        host_installation_id?: string;
      };
      const hostId = p.host_installation_id?.trim();
      if (!hostId) return false;
      this.handlers.onHostUnlinked?.({
        hostInstallationId: hostId,
        atMs: p.at_ms,
      });
      return true;
    }

    // Other durable kinds (session lifecycle, friend_request_updated, etc.):
    // advance cursor; optional handlers not required.
    return true;
  }
}
