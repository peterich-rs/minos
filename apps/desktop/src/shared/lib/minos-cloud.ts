/**
 * Minos backend HTTP client for Desktop account session + host cloud bind.
 *
 * Mirrors `apps/web/src/lib/minos.ts` auth/host paths (D01 dual session,
 * D02 same-account host link). Device role is `desktop-console`.
 */

const DEFAULT_BACKEND_URL = "http://127.0.0.1:8787";

export type AuthResponse = {
  account: { account_id: string; email: string };
  access_token: string;
  refresh_token: string;
  expires_in: number;
};

export type HostLinkResult = {
  hostInstallationId: string;
  hostInstallationToken: string;
  pairId: string;
  accountId: string;
  hostDisplayName: string;
  linkedAtMs: number;
};

export type ListedHost = {
  hostInstallationId: string;
  hostDisplayName: string;
  linkedAtMs: number;
  online: boolean;
  /** Durable last activity from hub; 0 when unknown. */
  lastSeenAtMs: number;
};

export class MinosCloudError extends Error {
  readonly status: number;
  readonly code: string | null;
  readonly payload: unknown;

  constructor(
    message: string,
    status: number,
    code: string | null = null,
    payload?: unknown,
  ) {
    super(message);
    this.name = "MinosCloudError";
    this.status = status;
    this.code = code;
    this.payload = payload;
  }
}

export function backendHttpBase(): string {
  // import.meta.env is Vite-only; node:test has no env bag.
  const viteEnv =
    typeof import.meta !== "undefined"
      ? (import.meta as { env?: { VITE_MINOS_BACKEND_URL?: string } }).env
      : undefined;
  const raw = viteEnv?.VITE_MINOS_BACKEND_URL ?? DEFAULT_BACKEND_URL;
  return raw.replace(/\/+$/, "");
}

function deviceHeaders(
  deviceId: string,
  accessToken?: string,
): Record<string, string> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "x-device-id": deviceId,
    "x-device-role": "desktop-console",
    "x-device-name": "Minos Desktop",
  };
  if (accessToken) {
    headers.authorization = `Bearer ${accessToken}`;
  }
  return headers;
}

async function parseErrorPayload(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null;
  }
}

function errorFromPayload(
  status: number,
  payload: unknown,
): MinosCloudError {
  if (payload && typeof payload === "object") {
    const record = payload as Record<string, unknown>;
    const nested = record.error;
    if (nested && typeof nested === "object") {
      const nestedRecord = nested as Record<string, unknown>;
      const code =
        typeof nestedRecord.code === "string" ? nestedRecord.code : null;
      const message =
        typeof nestedRecord.message === "string"
          ? nestedRecord.message
          : code ?? `request failed (${status})`;
      return new MinosCloudError(message, status, code, payload);
    }
    if (typeof record.message === "string") {
      return new MinosCloudError(record.message, status, null, payload);
    }
  }
  return new MinosCloudError(`request failed (${status})`, status, null, payload);
}

async function requestJson<T>(
  path: string,
  init: RequestInit,
): Promise<T> {
  const response = await fetch(`${backendHttpBase()}${path}`, init);
  if (!response.ok) {
    const payload = await parseErrorPayload(response);
    throw errorFromPayload(response.status, payload);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return response.json() as Promise<T>;
}

/** Exchange Supabase access token → Minos AuthResp (same shape as login). */
export async function exchangeSupabaseSession(
  deviceId: string,
  supabaseAccessToken: string,
  deviceName = "Minos Desktop",
): Promise<AuthResponse> {
  return requestJson<AuthResponse>("/v1/auth/supabase", {
    method: "POST",
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({
      access_token: supabaseAccessToken,
      device_name: deviceName,
    }),
  });
}

export async function refreshSession(
  deviceId: string,
  refreshToken: string,
): Promise<{
  access_token: string;
  refresh_token: string;
  expires_in: number;
}> {
  return requestJson("/v1/auth/refresh", {
    method: "POST",
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({ refresh_token: refreshToken }),
  });
}

/** Revoke Minos refresh token. Best-effort callers should catch errors. */
export async function logoutSession(
  deviceId: string,
  accessToken: string,
  refreshToken: string,
): Promise<void> {
  await requestJson<void>("/v1/auth/logout", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ refresh_token: refreshToken }),
  });
}

type ResponseEnvelope<T> = { data: T };

export async function linkHost(
  deviceId: string,
  accessToken: string,
  body: {
    installationId: string;
    nonce: string;
    publicKey: string;
    signature: string;
    hostDisplayName: string;
  },
): Promise<HostLinkResult> {
  const envelope = await requestJson<
    ResponseEnvelope<{
      host_installation_id: string;
      host_installation_token: string;
      link: {
        pair_id: string;
        account_id: string;
        host_display_name: string;
        linked_at_ms: number;
      };
    }>
  >("/v1/hosts/link", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({
      installation_id: body.installationId,
      nonce: body.nonce,
      public_key: body.publicKey,
      signature: body.signature,
      host_display_name: body.hostDisplayName,
    }),
  });
  const data = envelope.data;
  return {
    hostInstallationId: data.host_installation_id,
    hostInstallationToken: data.host_installation_token,
    pairId: data.link.pair_id,
    accountId: data.link.account_id,
    hostDisplayName: data.link.host_display_name,
    linkedAtMs: data.link.linked_at_ms,
  };
}

export async function unlinkHost(
  deviceId: string,
  accessToken: string,
  hostInstallationId: string,
): Promise<void> {
  await requestJson<void>("/v1/hosts/unlink", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ host_installation_id: hostInstallationId }),
  });
}

export async function listHosts(
  deviceId: string,
  accessToken: string,
): Promise<ListedHost[]> {
  const envelope = await requestJson<
    ResponseEnvelope<{
      hosts: Array<{
        host_installation_id: string;
        host_display_name: string;
        linked_at_ms: number;
        online: boolean;
        last_seen_at_ms?: number;
      }>;
    }>
  >("/v1/hosts", {
    method: "GET",
    headers: deviceHeaders(deviceId, accessToken),
  });
  return envelope.data.hosts.map((h) => ({
    hostInstallationId: h.host_installation_id,
    hostDisplayName: h.host_display_name,
    linkedAtMs: h.linked_at_ms,
    online: h.online,
    lastSeenAtMs: h.last_seen_at_ms ?? 0,
  }));
}

// ─── Multi-end IM (Desktop workbench → Hub) ─────────────────────────────

export async function upsertConversation(
  deviceId: string,
  accessToken: string,
  input: {
    conversationId: string;
    title: string;
    memberAccountIds?: string[];
    agentIds?: string[];
  },
): Promise<{ conversationId: string }> {
  return requestJson<{ conversation_id: string }>("/v1/conversations/upsert", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({
      conversation_id: input.conversationId,
      title: input.title,
      member_account_ids: input.memberAccountIds ?? [],
      agent_ids: input.agentIds ?? [],
    }),
  }).then((r) => ({ conversationId: r.conversation_id }));
}

export async function sendConversationMessage(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  input: {
    text: string;
    clientMessageId?: string;
    replyToMessageId?: string;
    /** Client clock for display/debug only (not ordering authority). */
    clientSentAtMs?: number;
    /** @deprecated Prefer clientSentAtMs. */
    createdAtMs?: number;
    /**
     * host_projection = dual-write of already-run Host messages (no agent dispatch).
     * Defaults server-side to client_live.
     */
    messageSource?: "client_live" | "host_projection" | "system";
  },
): Promise<{ messageId: string }> {
  const body: Record<string, unknown> = { text: input.text };
  if (input.clientMessageId) body.client_message_id = input.clientMessageId;
  if (input.replyToMessageId) body.reply_to_message_id = input.replyToMessageId;
  if (input.messageSource) body.message_source = input.messageSource;
  const clientTs = input.clientSentAtMs ?? input.createdAtMs;
  if (clientTs != null && clientTs > 0) {
    body.client_sent_at_ms = clientTs;
  }
  const resp = await requestJson<{ message_id: string }>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify(body),
    },
  );
  return { messageId: resp.message_id };
}

export type CloudAgentSummary = {
  agentId: string;
  ownerAccountId: string;
  name: string;
  description: string;
  runtimeAgent: string;
  model: string;
  workspacePath?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

function mapAgentSummary(raw: {
  agent_id: string;
  owner_account_id: string;
  name: string;
  description: string;
  runtime_agent: string;
  model: string;
  workspace_path?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
}): CloudAgentSummary {
  return {
    agentId: raw.agent_id,
    ownerAccountId: raw.owner_account_id,
    name: raw.name,
    description: raw.description,
    runtimeAgent: raw.runtime_agent,
    model: raw.model,
    workspacePath: raw.workspace_path,
    createdAtMs: raw.created_at_ms,
    updatedAtMs: raw.updated_at_ms,
  };
}

/** List cloud agents owned by the account. */
export async function listCloudAgents(
  deviceId: string,
  accessToken: string,
): Promise<CloudAgentSummary[]> {
  const resp = await requestJson<{ agents: Array<Parameters<typeof mapAgentSummary>[0]> }>(
    "/v1/agents/query",
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  );
  return (resp.agents ?? []).map(mapAgentSummary);
}

/**
 * Ensure a stable Host/Desktop runtime agent exists for local bin → cloud id.
 * Idempotent per (account, runtime_agent) with description `minos:host-runtime`.
 */
export async function ensureHostRuntimeAgent(
  deviceId: string,
  accessToken: string,
  input: {
    runtimeAgent: string;
    name?: string;
    model?: string;
    workspacePath?: string | null;
  },
): Promise<CloudAgentSummary> {
  const raw = await requestJson<Parameters<typeof mapAgentSummary>[0]>(
    "/v1/agents/ensure-host-runtime",
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({
        runtime_agent: input.runtimeAgent,
        name: input.name,
        model: input.model ?? "",
        workspace_path: input.workspacePath ?? undefined,
      }),
    },
  );
  return mapAgentSummary(raw);
}

export async function addAgentToConversation(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  agentId: string,
): Promise<void> {
  await requestJson<void>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/agents/add`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({ agent_id: agentId }),
    },
  );
}

export type HubReactionGroup = {
  emoji: string;
  count: number;
  reactedByMe: boolean;
  actors: Array<{ actorId: string; actorKind: string; displayName: string }>;
};

export type HubChatMessage = {
  messageId: string;
  conversationId: string;
  text: string;
  createdAtMs: number;
  /** Per-conversation Hub ordering key; undefined when absent (never pseudo-0). */
  messageSeq?: number;
  senderType: "user" | "agent";
  senderAccountId: string;
  senderMinosId: string;
  senderDisplayName: string;
  replyToMessageId?: string | null;
  recalledAtMs?: number | null;
  /** Viewer-resolved reaction aggregates from messages/query. */
  reactions?: HubReactionGroup[];
};

function mapHubChatMessage(raw: {
  message_id: string;
  conversation_id: string;
  text: string;
  created_at_ms: number;
  message_seq?: number;
  sender_type?: string;
  sender: {
    account_id: string;
    minos_id: string;
    display_name: string;
  };
  reply_to?: { message_id: string } | null;
  recalled_at_ms?: number | null;
  reactions?: Array<{
    emoji: string;
    count: number;
    reacted_by_me?: boolean;
    actors?: Array<{
      actor_id: string;
      actor_kind: string;
      display_name: string;
    }>;
  }>;
}): HubChatMessage {
  return {
    messageId: raw.message_id,
    conversationId: raw.conversation_id,
    text: raw.text,
    createdAtMs: raw.created_at_ms,
    // Missing message_seq stays undefined — never coerce to 0 (pseudo-order).
    messageSeq:
      raw.message_seq != null && Number.isFinite(raw.message_seq)
        ? raw.message_seq
        : undefined,
    senderType: raw.sender_type === "agent" ? "agent" : "user",
    senderAccountId: raw.sender.account_id,
    senderMinosId: raw.sender.minos_id,
    senderDisplayName: raw.sender.display_name,
    replyToMessageId: raw.reply_to?.message_id ?? null,
    recalledAtMs: raw.recalled_at_ms ?? null,
    reactions: (raw.reactions ?? []).map((g) => ({
      emoji: g.emoji,
      count: g.count,
      reactedByMe: Boolean(g.reacted_by_me),
      actors: (g.actors ?? []).map((a) => ({
        actorId: a.actor_id,
        actorKind: a.actor_kind,
        displayName: a.display_name,
      })),
    })),
  };
}

/** Account-scoped conversation digest from POST /v1/conversations/query. */
export type HubConversationListItem = {
  conversationId: string;
  title: string;
  preview: string | null;
  lastMessageAtMs: number;
  unreadCount: number;
  unreadMentionCount: number;
  kind: string;
  memberCount: number;
};

/**
 * Account-scoped Hub inbox digests (multi-end SSOT for rail unread/preview).
 * Single-flight hydrate source for HubDigestCache — not per-project.
 */
export async function listHubConversations(
  deviceId: string,
  accessToken: string,
): Promise<HubConversationListItem[]> {
  const resp = await requestJson<{
    conversations?: Array<{
      conversation_id: string;
      kind?: string;
      title?: string;
      member_count?: number;
      last_message_preview?: string | null;
      last_message_at_ms?: number;
      unread_count?: number;
      unread_mention_count?: number;
    }>;
  }>("/v1/conversations/query", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({}),
  });
  return (resp.conversations ?? []).map((c) => ({
    conversationId: c.conversation_id,
    title: c.title ?? "Conversation",
    preview: c.last_message_preview ?? null,
    lastMessageAtMs: c.last_message_at_ms ?? 0,
    unreadCount: c.unread_count ?? 0,
    unreadMentionCount: c.unread_mention_count ?? 0,
    kind: c.kind ?? "group",
    memberCount: c.member_count ?? 0,
  }));
}

/** Page result for Hub cold-path messages/query (gap fill / older pages). */
export type HubMessagePage = {
  messages: HubChatMessage[];
  /** Cursor for next older page (`before_seq`); null when no more history. */
  nextBeforeSeq: number | null;
};

/**
 * Cold-read hub conversation timeline (Mobile / multi-end).
 * Gap API: `before_seq` / `after_seq` keyset on per-conversation message_seq.
 */
export async function listHubConversationMessages(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  opts?: { beforeSeq?: number; afterSeq?: number; limit?: number },
): Promise<HubMessagePage> {
  const resp = await requestJson<{
    messages: Array<Parameters<typeof mapHubChatMessage>[0]>;
    next_before_seq?: number | null;
  }>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages/query`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({
        before_seq: opts?.beforeSeq,
        after_seq: opts?.afterSeq,
        limit: opts?.limit ?? 100,
      }),
    },
  );
  const messages = (resp.messages ?? []).map(mapHubChatMessage);
  return {
    messages,
    nextBeforeSeq:
      resp.next_before_seq === undefined ? null : resp.next_before_seq,
  };
}

/** Hub mark-read (Linked inbox unread). */
export async function markHubConversationRead(
  deviceId: string,
  accessToken: string,
  conversationId: string,
): Promise<{ lastReadSeq: number | null; lastReadAtMs: number | null }> {
  const resp = await requestJson<{
    last_read_seq?: number | null;
    last_read_at_ms?: number | null;
  }>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/read`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  );
  return {
    lastReadSeq: resp.last_read_seq ?? null,
    lastReadAtMs: resp.last_read_at_ms ?? null,
  };
}

/** Cloud reaction toggle (Hub message ids only). Aggregate is SSOT. */
export async function toggleHubReaction(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  messageId: string,
  emoji: string,
  /** Required B6: outbox entry id; same value on retry for event_id idempotency. */
  clientOpId: string,
): Promise<{
  messageId: string;
  conversationId: string;
  reactions: Array<{
    emoji: string;
    count: number;
    reactedByMe: boolean;
    actors: Array<{ actorId: string; actorKind: string; displayName: string }>;
  }>;
  action: string;
}> {
  const opId = clientOpId.trim();
  if (!opId) {
    throw new Error("client_op_id is required for reaction toggle");
  }
  const resp = await requestJson<{
    message_id: string;
    conversation_id: string;
    action: string;
    reactions?: Array<{
      emoji: string;
      count: number;
      reacted_by_me: boolean;
      actors?: Array<{
        actor_id: string;
        actor_kind: string;
        display_name: string;
      }>;
    }>;
  }>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages/${encodeURIComponent(messageId)}/reactions/toggle`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({ emoji, client_op_id: opId }),
    },
  );
  return {
    messageId: resp.message_id,
    conversationId: resp.conversation_id,
    action: resp.action,
    reactions: (resp.reactions ?? []).map((g) => ({
      emoji: g.emoji,
      count: g.count,
      reactedByMe: g.reacted_by_me,
      actors: (g.actors ?? []).map((a) => ({
        actorId: a.actor_id,
        actorKind: a.actor_kind,
        displayName: a.display_name,
      })),
    })),
  };
}

/** Hub multi-end recall (sender within window). */
export async function recallHubMessage(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  messageId: string,
): Promise<HubChatMessage> {
  const resp = await requestJson<Parameters<typeof mapHubChatMessage>[0]>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages/${encodeURIComponent(messageId)}/recall`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  );
  return mapHubChatMessage(resp);
}

export async function createWsTicket(
  deviceId: string,
  accessToken: string,
): Promise<{ ticket: string; gatewayUrl: string; expiresAtMs: number }> {
  // Backend wraps as { data: { ticket, gateway_url, expires_at_ms }, meta }.
  const envelope = await requestJson<{
    data: {
      ticket: string;
      gateway_url: string;
      expires_at_ms: number;
    };
  }>("/v1/realtime/ws-ticket", {
    method: "POST",
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ installation_id: deviceId }),
  });
  const data = envelope.data;
  // gateway_url is often a path like `/ws/client?ticket=…` (ticket already embedded).
  return {
    ticket: data.ticket,
    gatewayUrl: data.gateway_url,
    expiresAtMs: data.expires_at_ms,
  };
}

/** Absolute WS URL for formal client gateway from ticket response + HTTP base. */
export function hubClientWsUrl(gatewayUrl: string, ticket: string): string {
  const httpBase = backendHttpBase();
  let pathOrUrl = gatewayUrl.trim();
  // Relative path → absolute against backend origin.
  if (pathOrUrl.startsWith("/")) {
    const origin = httpBase
      .replace(/^https:/i, "wss:")
      .replace(/^http:/i, "ws:");
    pathOrUrl = `${origin.replace(/\/+$/, "")}${pathOrUrl}`;
  } else if (pathOrUrl.startsWith("https://")) {
    pathOrUrl = `wss://${pathOrUrl.slice("https://".length)}`;
  } else if (pathOrUrl.startsWith("http://")) {
    pathOrUrl = `ws://${pathOrUrl.slice("http://".length)}`;
  } else if (!pathOrUrl.startsWith("ws://") && !pathOrUrl.startsWith("wss://")) {
    const origin = httpBase
      .replace(/^https:/i, "wss:")
      .replace(/^http:/i, "ws:");
    pathOrUrl = `${origin.replace(/\/+$/, "")}/ws/client?ticket=${encodeURIComponent(ticket)}`;
  }
  // If ticket not already in query, append it.
  if (!/[?&]ticket=/.test(pathOrUrl)) {
    const sep = pathOrUrl.includes("?") ? "&" : "?";
    pathOrUrl = `${pathOrUrl}${sep}ticket=${encodeURIComponent(ticket)}`;
  }
  return pathOrUrl;
}

/** Dual-write an agent chat bubble (idempotent via client_message_id). */
export async function sendAgentConversationMessage(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  input: {
    agentId: string;
    text: string;
    clientMessageId?: string;
    replyToMessageId?: string | null;
    agentSessionId?: string | null;
    clientSentAtMs?: number;
    /** @deprecated Prefer clientSentAtMs. */
    createdAtMs?: number;
    messageSource?: "client_live" | "host_projection" | "system";
  },
): Promise<{ messageId: string }> {
  const body: Record<string, unknown> = {
    agent_id: input.agentId,
    text: input.text,
  };
  if (input.clientMessageId) body.client_message_id = input.clientMessageId;
  if (input.replyToMessageId) body.reply_to_message_id = input.replyToMessageId;
  if (input.agentSessionId) body.agent_session_id = input.agentSessionId;
  if (input.messageSource) body.message_source = input.messageSource;
  const clientTs = input.clientSentAtMs ?? input.createdAtMs;
  if (clientTs != null && clientTs > 0) {
    body.client_sent_at_ms = clientTs;
  }
  const resp = await requestJson<{ message_id: string }>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/agents/message`,
    {
      method: "POST",
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify(body),
    },
  );
  return { messageId: resp.message_id };
}
