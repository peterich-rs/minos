/**
 * Desktop Account client → minos-backend formal `/ws/client` realtime.
 *
 * Phase 4 Sync Engine:
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
} from "@/shared/lib/hub-cursors";
import {
  createWsTicket,
  hubClientWsUrl,
  type HubChatMessage,
} from "@/shared/lib/minos-cloud";

export type HubRealtimeSyncState =
  | "disconnected"
  | "connecting"
  | "syncing"
  | "live"
  | "error";

type DurableMessagePayload = {
  kind?: string;
  account_id?: string;
  conversation_id?: string;
  message_id?: string;
  at_ms?: number;
  message?: {
    message_id: string;
    conversation_id: string;
    text: string;
    created_at_ms: number;
    sender_type?: string;
    sender: {
      account_id: string;
      minos_id: string;
      display_name: string;
    };
    reply_to?: { message_id: string } | null;
    recalled_at_ms?: number | null;
  };
};

export type HubRealtimeHandlers = {
  onChatMessage: (message: HubChatMessage) => void;
  /** Multi-end recall: remove or mark recalled in Hub timeline. */
  onChatMessageRecalled?: (message: HubChatMessage) => void;
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
  onConnectionChange?: (state: HubRealtimeSyncState) => void;
  /**
   * Cursor too old for topic — clear projection for conversation topics and
   * cold-pull snapshot page, then reset cursor.
   */
  onSnapshotRequired?: (topic: string) => void;
};

function mapMessage(
  raw: NonNullable<DurableMessagePayload["message"]>,
): HubChatMessage {
  return {
    messageId: raw.message_id,
    conversationId: raw.conversation_id,
    text: raw.text,
    createdAtMs: raw.created_at_ms,
    senderType: raw.sender_type === "agent" ? "agent" : "user",
    senderAccountId: raw.sender.account_id,
    senderMinosId: raw.sender.minos_id,
    senderDisplayName: raw.sender.display_name,
    replyToMessageId: raw.reply_to?.message_id ?? null,
    recalledAtMs: raw.recalled_at_ms ?? null,
  };
}

const APPEND_KINDS = new Set([
  "account_conversation_message_appended",
  "AccountConversationMessageAppended",
  "conversation_message_appended",
  "ConversationMessageAppended",
]);

const RECALL_KINDS = new Set([
  "account_conversation_message_recalled",
  "AccountConversationMessageRecalled",
  "conversation_message_recalled",
  "ConversationMessageRecalled",
]);

const REACTION_KINDS = new Set([
  "conversation_message_reaction_updated",
  "ConversationMessageReactionUpdated",
]);

export class HubRealtimeSession {
  private ws: WebSocket | null = null;
  private stopped = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempt = 0;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  /** When true, skip starting ping (document hidden); resume on show. */
  private pingPaused = false;
  private readonly handlers: HubRealtimeHandlers;
  private auth: {
    deviceId: string;
    accessToken: string;
    accountId: string;
  } | null = null;
  private syncState: HubRealtimeSyncState = "disconnected";
  /** Desired conversation topics (open windows); re-subscribed on reconnect. */
  private conversationIds = new Set<string>();
  private cursors: TopicCursorMap = loadTopicCursors();
  /** Topics we still expect SubscribeAck for after a Subscribe batch. */
  private pendingSubscribeTopics = new Set<string>();

  constructor(handlers: HubRealtimeHandlers) {
    this.handlers = handlers;
  }

  get state(): HubRealtimeSyncState {
    return this.syncState;
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
   * resume_after when Live/Syncing.
   */
  subscribeConversation(conversationId: string): void {
    const id = conversationId?.trim();
    if (!id) return;
    this.conversationIds.add(id);
    this.sendSubscribe([conversationTopic(id)]);
  }

  /** Leave conversation topic when window closed (optional). */
  unsubscribeConversation(conversationId: string): void {
    const id = conversationId?.trim();
    if (!id) return;
    this.conversationIds.delete(id);
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

  private setState(state: HubRealtimeSyncState): void {
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
    for (const id of this.conversationIds) {
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
      const url = hubClientWsUrl(ticket.gatewayUrl, ticket.ticket);
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
      console.warn("[hub-realtime] connect failed", error);
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
      console.warn("[hub-realtime] subscription denied", frame);
      return;
    }

    if (type === "pong") {
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

    // Some serializers nest kind on payload only.
    if (frame.payload && typeof frame.payload === "object" && "kind" in frame.payload) {
      const p = frame.payload as DurableMessagePayload;
      this.handleDurable(p.kind, p);
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

    if (APPEND_KINDS.has(eventKind)) {
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
        console.warn("[hub-realtime] failed to map chat message", error);
        return false;
      }
    }

    if (RECALL_KINDS.has(eventKind)) {
      try {
        if (payload.message) {
          this.handlers.onChatMessageRecalled?.(mapMessage(payload.message));
          return true;
        }
        if (payload.message_id && payload.conversation_id) {
          // Minimal recall payload without full message body.
          this.handlers.onChatMessageRecalled?.({
            messageId: payload.message_id,
            conversationId: payload.conversation_id,
            text: "",
            createdAtMs: payload.at_ms ?? 0,
            senderType: "user",
            senderAccountId: "",
            senderMinosId: "",
            senderDisplayName: "",
            replyToMessageId: null,
            recalledAtMs: payload.at_ms ?? Date.now(),
          });
          return true;
        }
        return false;
      } catch (error) {
        console.warn("[hub-realtime] failed to map recall", error);
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

    // Other durable kinds (session lifecycle, etc.): ignore but advance.
    return true;
  }
}
