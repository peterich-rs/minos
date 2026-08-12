/**
 * Pure durable-frame mappers for Account `/ws/client` realtime.
 * Kept free of WebSocket session lifecycle (size / test isolation).
 */

import type { CloudChatMessage } from "@/shared/lib/minos-cloud";

export type DurableMessagePayload = {
  kind?: string;
  account_id?: string;
  conversation_id?: string;
  message_id?: string;
  at_ms?: number;
  /** Account thin digest fields (no nested full message). */
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

export function mapMessage(
  raw: NonNullable<DurableMessagePayload["message"]>,
): CloudChatMessage {
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
export const CONVERSATION_APPEND_KINDS = new Set([
  "conversation_message_appended",
  "ConversationMessageAppended",
]);

export const CONVERSATION_RECALL_KINDS = new Set([
  "conversation_message_recalled",
  "ConversationMessageRecalled",
]);

/** Account topic T2 thin digest (inbox/rail only). */
export const ACCOUNT_APPEND_KINDS = new Set([
  "account_conversation_message_appended",
  "AccountConversationMessageAppended",
]);

export const ACCOUNT_RECALL_KINDS = new Set([
  "account_conversation_message_recalled",
  "AccountConversationMessageRecalled",
]);

export const REACTION_KINDS = new Set([
  "conversation_message_reaction_updated",
  "ConversationMessageReactionUpdated",
]);

export function mapAccountDigest(
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

