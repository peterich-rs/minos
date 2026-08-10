/**
 * Desktop → Hub IM helpers (conversation shell + user-message Outbox).
 *
 * Collaboration path (Account live only):
 * - User messages: `client_live` via Account WS `AppendMessage` (no REST write).
 * - Bot delivery: Hub Agent inbox → BotInboxDelivery on the bound Host.
 * - Agent final text: Host AppendBotMessage (or TurnCompletionProjector offline).
 *
 * Conversation shell upsert + host-runtime agent resolve remain.
 */

import { useAccountStore } from "@/store/account-store";
import {
  addAgentToConversation,
  ensureHostRuntimeAgent,
  respondHubApproval,
  sendAgentConversationMessage,
  toggleHubReaction,
  upsertConversation,
} from "@/shared/lib/minos-cloud";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import { appendMessageOnCloud } from "@/shared/lib/im-cloud-bridge";
import {
  displayNameForRuntime,
  isCanonicalAgentResultId,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";
import { sessionIdFromAgentResultId } from "@/shared/lib/cloud-timeline";
import {
  enqueueAgentResult,
  enqueueApprovalResolve,
  enqueueReactionToggle,
  enqueueUserMessage,
  earliestPendingAttemptAt,
  getOutboxEntry,
  initImOutbox,
  isAcked,
  listDuePending,
  listDuePendingLanes,
  markAcked,
  markFailed,
  markInflight,
  outboxLaneKey,
  reclaimStaleInflight,
  type ImOutboxEntry,
} from "@/shared/lib/im-outbox";
import { daemonApi } from "@/shared/lib/daemon";
import { toast } from "@/shared/lib/toast";
import type { TimelineMessage } from "@/shared/lib/mock-data";

export {
  displayNameForRuntime,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";

/** In-memory runtime → cloud agent_id for this process. */
const runtimeAgentIdCache = new Map<string, string>();

/** Track acked / seen hub message ids (user outbox + inbound echo). */
const projectedMessageIds = new Set<string>();
const MAX_PROJECTED_CACHE = 4000;

/** Per-lane drain chains — different conversations / intent classes parallelize. */
const laneChains = new Map<string, Promise<void>>();
let outboxWorkerTimer: ReturnType<typeof setTimeout> | null = null;

/** Last successful reaction toggle payload by client op id (for waiters). */
const reactionResults = new Map<
  string,
  {
    messageId: string;
    conversationId: string;
    action: string;
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
  }
>();

function cloudAuth(): { deviceId: string; accessToken: string } | null {
  const { deviceId, session } = useAccountStore.getState();
  if (!session?.accessToken?.trim()) return null;
  return { deviceId, accessToken: session.accessToken };
}

function normalizeRuntime(runtime: string | null | undefined): string | null {
  return normalizeHostRuntime(runtime);
}

function rememberProjected(messageId: string): void {
  projectedMessageIds.add(messageId);
  if (projectedMessageIds.size > MAX_PROJECTED_CACHE) {
    const drop = projectedMessageIds.size - MAX_PROJECTED_CACHE;
    let i = 0;
    for (const id of projectedMessageIds) {
      projectedMessageIds.delete(id);
      i += 1;
      if (i >= drop) break;
    }
  }
}

/** Mark message as already on Hub (inbound / send ack). */
export function markMessageProjected(messageId: string): void {
  const id = messageId.trim();
  if (!id) return;
  rememberProjected(id);
  void markAcked(id);
}

/**
 * Resolve local agent bin name → cloud agent_id via ensure-host-runtime.
 * Never treats the bin name itself as a cloud agent_id.
 */
export async function resolveCloudAgentId(
  runtimeAgent: string,
): Promise<string | null> {
  const runtime = normalizeRuntime(runtimeAgent);
  if (!runtime) return null;
  const cached = runtimeAgentIdCache.get(runtime);
  if (cached) return cached;

  const auth = cloudAuth();
  if (!auth) return null;

  try {
    const agent = await ensureHostRuntimeAgent(auth.deviceId, auth.accessToken, {
      runtimeAgent: runtime,
      name: displayNameForRuntime(runtime),
    });
    runtimeAgentIdCache.set(runtime, agent.agentId);
    return agent.agentId;
  } catch (error) {
    console.warn("[im-cloud-sync] ensure host runtime agent failed", runtime, error);
    return null;
  }
}

export async function resolveCloudAgentIds(
  runtimes: Array<string | null | undefined>,
): Promise<string[]> {
  const unique = new Set<string>();
  for (const r of runtimes) {
    const n = normalizeRuntime(r);
    if (n) unique.add(n);
  }
  const ids: string[] = [];
  for (const runtime of unique) {
    const id = await resolveCloudAgentId(runtime);
    if (id) ids.push(id);
  }
  return ids;
}

/** Register / refresh a Desktop work conversation on the hub (shell + roster). */
export async function syncConversationToCloud(input: {
  conversationId: string;
  title: string;
  memberAccountIds?: string[];
  /** Local runtime names (codex/claude/…); resolved to cloud agent ids. */
  agentRuntimes?: Array<string | null | undefined>;
  /** Pre-resolved cloud agent ids (optional; merged with agentRuntimes). */
  agentIds?: string[];
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  const title = input.title.trim();
  if (!title || !input.conversationId.trim()) return;

  const fromRuntimes = await resolveCloudAgentIds(input.agentRuntimes ?? []);
  const agentIds = [
    ...new Set([...(input.agentIds ?? []), ...fromRuntimes].filter(Boolean)),
  ];

  try {
    await upsertConversation(auth.deviceId, auth.accessToken, {
      conversationId: input.conversationId,
      title,
      memberAccountIds: input.memberAccountIds ?? [],
      agentIds,
    });
  } catch (error) {
    console.warn("[im-cloud-sync] upsert conversation failed", error);
  }
}

/**
 * Attach host-runtime agents to an existing hub conversation without touching
 * the title (used when starting a session mid-conversation).
 */
export async function attachAgentsToConversationCloud(input: {
  conversationId: string;
  agentRuntimes: Array<string | null | undefined>;
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  if (!input.conversationId.trim()) return;
  const agentIds = await resolveCloudAgentIds(input.agentRuntimes);
  for (const agentId of agentIds) {
    try {
      await addAgentToConversation(
        auth.deviceId,
        auth.accessToken,
        input.conversationId,
        agentId,
      );
    } catch (error) {
      console.warn(
        "[im-cloud-sync] add agent to conversation failed",
        agentId,
        error,
      );
    }
  }
}

const RUNTIMES_FROM_ID = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
] as const;

export type HubUserMessageAck = {
  messageId: string;
  messageSeq: number;
  conversationId: string;
};

const userMessageAcks = new Map<string, HubUserMessageAck>();

async function postUserMessageFromOutbox(
  entry: ImOutboxEntry,
): Promise<HubUserMessageAck> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("Not signed in — message not synced to cloud");
  }

  // Never upsert Hub with the "Conversation" placeholder — that clobbers real
  // titles on every host_projection message. Only push a real title; agents-only
  // attach uses attachAgentsToConversationCloud when title is missing.
  const realTitle = entry.title?.trim() ?? "";
  const isPlaceholder =
    !realTitle || realTitle.toLowerCase() === "conversation";
  if (!isPlaceholder) {
    await syncConversationToCloud({
      conversationId: entry.conversationId,
      title: realTitle,
      agentRuntimes: entry.agentRuntimes,
    });
  } else if ((entry.agentRuntimes?.length ?? 0) > 0) {
    await attachAgentsToConversationCloud({
      conversationId: entry.conversationId,
      agentRuntimes: entry.agentRuntimes ?? [],
    });
  }

  // Linked Hub-first sends use client_live so Hub can @-dispatch.
  // Entries without explicit source default to client_live.
  const messageSource = entry.messageSource ?? "client_live";

  // Account WS AppendMessage only (no REST collaboration write path).
  // Wait for ChatSendAck before treating outbox as success.
  // Socket/timeout → retry via outbox; definitive nack fails the entry.
  if (messageSource !== "client_live") {
    // host_projection / system user rows are not collaboration writes here.
    // Agent final text uses postAgentResultFromOutbox.
    throw new Error(
      `Unsupported user-message source for Hub uplink: ${messageSource}`,
    );
  }

  const wsResult = await appendMessageOnCloud({
    clientOperationId: entry.clientMessageId,
    conversationId: entry.conversationId,
    text: entry.text,
    replyToMessageId: entry.replyToMessageId,
    mentions: entry.mentions,
  });
  if (wsResult.ok) {
    return {
      messageId: wsResult.messageId,
      messageSeq: wsResult.messageSeq,
      conversationId: wsResult.conversationId,
    };
  }
  if (wsResult.reason === "nack") {
    throw new Error(
      `ChatSendNack: ${wsResult.code ?? "nack"}${
        wsResult.message ? ` — ${wsResult.message}` : ""
      }`,
    );
  }
  throw new Error(
    wsResult.reason === "timeout"
      ? "AppendMessage timed out waiting for ChatSendAck"
      : "AppendMessage unavailable (Account WS not live)",
  );
}

async function postAgentResultFromOutbox(entry: ImOutboxEntry): Promise<void> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("Not signed in — agent result not synced to cloud");
  }

  let agentId = entry.agentId?.trim() || null;
  if (!agentId) {
    const runtime =
      entry.agentRuntimes?.find((r) => !!r?.trim()) ??
      RUNTIMES_FROM_ID.find((r) =>
        entry.clientMessageId.toLowerCase().includes(r),
      ) ??
      null;
    if (runtime) {
      agentId = await resolveCloudAgentId(runtime);
    }
  }
  if (!agentId) {
    throw new Error("agent_result uplink: no cloud agent id");
  }

  const sessionId =
    entry.agentSessionId?.trim() ||
    sessionIdFromAgentResultId(entry.clientMessageId) ||
    undefined;

  await sendAgentConversationMessage(
    auth.deviceId,
    auth.accessToken,
    entry.conversationId,
    {
      agentId,
      text: entry.text,
      clientMessageId: entry.clientMessageId,
      agentSessionId: sessionId,
      replyToMessageId: entry.replyToMessageId ?? undefined,
      messageSource: "host_projection",
    },
  );
}

export type HubReactionToggleResult = {
  messageId: string;
  conversationId: string;
  action: string;
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
};

async function postReactionToggleFromOutbox(
  entry: ImOutboxEntry,
): Promise<HubReactionToggleResult> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("not authenticated");
  }
  let payload: { messageId?: string; emoji?: string };
  try {
    payload = JSON.parse(entry.text) as { messageId?: string; emoji?: string };
  } catch {
    throw new Error("invalid_payload: reaction_toggle text is not JSON");
  }
  const messageId = payload.messageId?.trim() ?? "";
  const emoji = payload.emoji?.trim() ?? "";
  if (!messageId || !emoji) {
    throw new Error("invalid_payload: reaction_toggle requires messageId+emoji");
  }
  // clientMessageId == B6 client_op_id; retries reuse the same id.
  return toggleHubReaction(
    auth.deviceId,
    auth.accessToken,
    entry.conversationId,
    messageId,
    emoji,
    entry.clientMessageId,
  );
}

/**
 * Treat daemon/Hub "already resolved" as success so outbox retries after a
 * successful-but-unacked first attempt do not burn the entry.
 */
function isApprovalAlreadyResolvedError(error: unknown): boolean {
  const code =
    error && typeof error === "object" && "code" in error
      ? String((error as { code: unknown }).code ?? "").toLowerCase()
      : "";
  const msg = (error instanceof Error ? error.message : String(error)).toLowerCase();
  // Only treat explicit already-resolved / approval_not_found as success.
  // Broad "not found" would mask unrelated 404s and burn retries incorrectly.
  return (
    code.includes("already_resolved") ||
    code.includes("approval_already") ||
    code.includes("approval_not_found") ||
    msg.includes("already_resolved") ||
    msg.includes("already resolved") ||
    msg.includes("approval_already") ||
    msg.includes("approval_not_found") ||
    msg.includes("approval not found")
  );
}

/**
 * P3 branch by auth mode (no dual SSOT):
 * - Hub IM mode: POST /v1/approvals/respond + top-level client_request_id
 * - Local-only: daemon resolveApproval; decision JSON never carries client_request_id
 */
async function postApprovalResolveFromOutbox(
  entry: ImOutboxEntry,
): Promise<void> {
  let payload: {
    requestId?: string;
    sessionId?: string;
    decision?: string | Record<string, unknown>;
    /** Explicit route stamped at enqueue (`hub` | `daemon`). */
    route?: string;
  };
  try {
    payload = JSON.parse(entry.text) as typeof payload;
  } catch {
    throw new Error("invalid_payload: approval_resolve text is not JSON");
  }
  const requestId = payload.requestId?.trim() ?? "";
  const sessionId = payload.sessionId?.trim() ?? "";
  if (!requestId || !sessionId || payload.decision == null) {
    throw new Error(
      "invalid_payload: approval_resolve requires requestId+sessionId+decision",
    );
  }
  // Clean agent decision only — never nest client_request_id in decision JSON.
  const decision =
    typeof payload.decision === "string"
      ? { decision: payload.decision }
      : { ...payload.decision };
  delete (decision as Record<string, unknown>).client_request_id;

  const { session, authPhase } = useAccountStore.getState();
  const hubRoute =
    payload.route === "hub" ||
    (payload.route !== "daemon" &&
      isCloudImMode({
        authPhase,
        accessToken: session?.accessToken,
      }));

  try {
    if (hubRoute) {
      const auth = cloudAuth();
      if (!auth) {
        throw new Error("not authenticated");
      }
      // Top-level client_request_id = outbox logical op id (Intent Outbox).
      await respondHubApproval(auth.deviceId, auth.accessToken, {
        requestId,
        decision,
        clientRequestId: entry.clientMessageId,
      });
    } else {
      await daemonApi.resolveApproval(requestId, sessionId, decision);
    }
  } catch (error) {
    if (isApprovalAlreadyResolvedError(error)) {
      return;
    }
    throw error;
  }
}

async function postOutboxEntry(
  entry: ImOutboxEntry,
): Promise<HubReactionToggleResult | HubUserMessageAck | void> {
  switch (entry.kind) {
    case "user_message":
      return postUserMessageFromOutbox(entry);
    case "agent_result":
      await postAgentResultFromOutbox(entry);
      return;
    case "reaction_toggle":
      return postReactionToggleFromOutbox(entry);
    case "approval_resolve":
      await postApprovalResolveFromOutbox(entry);
      return;
    default: {
      const _exhaustive: never = entry.kind;
      throw new Error(`unknown outbox kind: ${_exhaustive}`);
    }
  }
}

/**
 * C5.1 single path: enqueue reaction_toggle then drain via lane worker.
 * Returns Hub aggregate for generation-gated apply (from worker side map).
 * Logical op id = clientOpId (= B6 client_op_id); row id = outbox:${clientOpId}.
 */
export async function syncReactionToggleToCloud(input: {
  conversationId: string;
  messageId: string;
  emoji: string;
  clientOpId: string;
}): Promise<HubReactionToggleResult | null> {
  const conversationId = input.conversationId.trim();
  const messageId = input.messageId.trim();
  const emoji = input.emoji.trim();
  const clientOpId = input.clientOpId.trim();
  if (!conversationId || !messageId || !emoji || !clientOpId) {
    throw new Error("invalid_payload: reaction_toggle missing fields");
  }
  if (await isAcked(clientOpId)) {
    return reactionResults.get(clientOpId) ?? null;
  }
  if (!cloudAuth()) {
    throw new Error("not authenticated");
  }

  const entry = await enqueueReactionToggle({
    conversationId,
    clientMessageId: clientOpId,
    text: JSON.stringify({ messageId, emoji }),
  });
  if (entry.status === "acked" || (await isAcked(clientOpId))) {
    return reactionResults.get(clientOpId) ?? null;
  }

  await waitForOutboxSettlement(clientOpId, {
    throwOnTerminal: true,
    silentToast: true,
  });
  return reactionResults.get(clientOpId) ?? null;
}

/**
 * C5.3 / P3: durable approval intent via Intent Outbox.
 *
 * - Hub authenticated: POST /v1/approvals/respond with top-level
 *   `client_request_id` (= clientOpId). Decision body is agent-facing only.
 * - Local / no hub: daemon `resolveApproval` for reachability; never stuff
 *   client_request_id into decision JSON.
 */
export async function syncApprovalResolve(input: {
  sessionId: string;
  requestId: string;
  decision: Record<string, unknown>;
  clientOpId: string;
  /** Force route; default derives from Hub IM mode at enqueue. */
  route?: "hub" | "daemon";
}): Promise<void> {
  const sessionId = input.sessionId.trim();
  const requestId = input.requestId.trim();
  const clientOpId = input.clientOpId.trim();
  if (!sessionId || !requestId || !clientOpId) {
    throw new Error("invalid_payload: approval_resolve missing fields");
  }
  if (await isAcked(clientOpId)) return;

  const decision = { ...input.decision };
  delete decision.client_request_id;

  const { session, authPhase } = useAccountStore.getState();
  const route =
    input.route ??
    (isCloudImMode({
      authPhase,
      accessToken: session?.accessToken,
    })
      ? "hub"
      : "daemon");

  if (route === "hub" && !cloudAuth()) {
    throw new Error("not authenticated");
  }

  const entry = await enqueueApprovalResolve({
    // Scope key for local storage only (not a Hub conversation id).
    conversationId: `approval-session:${sessionId}`,
    clientMessageId: clientOpId,
    text: JSON.stringify({
      requestId,
      sessionId,
      decision,
      route,
    }),
  });
  if (entry.status === "acked" || (await isAcked(clientOpId))) return;

  // Never flush outside the lane worker — preserves FIFO and intent lanes.
  await waitForOutboxSettlement(clientOpId, {
    throwOnTerminal: true,
    silentToast: true,
  });
}

/**
 * Enqueue a user timeline message then drain via per-conversation message lane.
 * Surfaces toast on terminal failure (multi-end intent must not silent-succeed).
 */
export async function syncUserMessageToCloud(input: {
  conversationId: string;
  messageId: string;
  text: string;
  /** When set, upsert conversation first so older local-only rows get a real title. */
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: Array<string | null | undefined>;
  createdAtMs?: number | null;
  /**
   * Write provenance. Linked Hub-first: `client_live` (default).
   * Rare host_projection only when replaying already-executed local rows.
   */
  messageSource?: "client_live" | "host_projection" | "system";
  /** Structured AppendMessage mentions (bot/account). */
  mentions?: ImOutboxEntry["mentions"];
}): Promise<HubUserMessageAck | null> {
  const auth = cloudAuth();
  if (!auth) return null;
  const text = input.text.trim();
  const messageId = input.messageId.trim();
  if (!text || !input.conversationId.trim() || !messageId) return null;
  if (await isAcked(messageId)) {
    return userMessageAcks.get(messageId) ?? null;
  }

  await enqueueUserMessage({
    conversationId: input.conversationId,
    clientMessageId: messageId,
    text,
    title: input.title,
    replyToMessageId: input.replyToMessageId,
    agentRuntimes: input.agentRuntimes,
    clientSentAtMs: input.createdAtMs,
    messageSource: input.messageSource ?? "client_live",
    mentions: input.mentions,
  });

  await waitForOutboxSettlement(messageId, { throwOnTerminal: true });
  return userMessageAcks.get(messageId) ?? null;
}

/**
 * Worker-only flush for one outbox row. Must not be called from hot-path
 * public APIs — always go through `flushImOutbox` / lane chains.
 */
async function flushOutboxEntry(
  entry: ImOutboxEntry,
  opts?: { silentToast?: boolean },
): Promise<void> {
  if (entry.status === "acked") return;
  if (await isAcked(entry.clientMessageId)) return;

  await markInflight(entry.clientMessageId);
  try {
    const result = await postOutboxEntry(entry);
    await markAcked(entry.clientMessageId);
    if (entry.kind === "user_message" || entry.kind === "agent_result") {
      rememberProjected(entry.clientMessageId);
    }
    if (entry.kind === "reaction_toggle" && result) {
      reactionResults.set(
        entry.clientMessageId,
        result as HubReactionToggleResult,
      );
    }
    if (entry.kind === "user_message" && result && "messageSeq" in result) {
      userMessageAcks.set(entry.clientMessageId, result as HubUserMessageAck);
    }
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    const status = await markFailed(entry.clientMessageId, msg);
    console.warn("[im-cloud-sync] outbox flush failed", entry.kind, error);
    const isMessageKind =
      entry.kind === "user_message" || entry.kind === "agent_result";
    if (!opts?.silentToast && isMessageKind) {
      if (status === "failed_terminal") {
        toast.error(
          "Cloud sync failed",
          `Message not visible on other devices: ${msg}`,
        );
      } else {
        toast.warning(
          "Cloud sync delayed",
          "Will retry sending this message to other devices.",
        );
      }
    }
  }
}

async function drainLaneKey(laneKey: string): Promise<void> {
  const prev = laneChains.get(laneKey) ?? Promise.resolve();
  let release!: () => void;
  const gate = new Promise<void>((r) => {
    release = r;
  });
  laneChains.set(
    laneKey,
    prev.then(() => gate).catch(() => gate),
  );
  await prev.catch(() => undefined);
  try {
    // Re-list so we only take this lane's current due head-run.
    const lanes = await listDuePendingLanes();
    const lane = lanes.find(
      (l) => l[0] != null && outboxLaneKey(l[0]) === laneKey,
    );
    if (!lane) return;
    for (const entry of lane) {
      await flushOutboxEntry(entry);
      if (!(await isAcked(entry.clientMessageId))) break;
    }
  } finally {
    release();
  }
}

/**
 * Drain due pending outbox rows with per-lane FIFO.
 *
 * Lanes: `message:{conv}` | `reaction:{conv}` | `approval:{scope}`.
 * Same lane is strict FIFO; different lanes run in parallel.
 * Public sync APIs must only enqueue + wait — never call flushOutboxEntry.
 */
export async function flushImOutbox(): Promise<void> {
  const lanes = await listDuePendingLanes();
  if (lanes.length === 0) return;
  await Promise.all(
    lanes.map(async (lane) => {
      const head = lane[0];
      if (!head) return;
      await drainLaneKey(outboxLaneKey(head));
    }),
  );
}

/**
 * Kick lane workers until this op is acked, terminal, or deadline.
 * Used by public sync APIs instead of direct flush (preserves FIFO).
 */
async function waitForOutboxSettlement(
  clientMessageId: string,
  opts?: {
    throwOnTerminal?: boolean;
    silentToast?: boolean;
    timeoutMs?: number;
  },
): Promise<"acked" | "failed_terminal" | "timeout"> {
  const id = clientMessageId.trim();
  const deadline = Date.now() + (opts?.timeoutMs ?? 120_000);
  scheduleOutboxWorker();
  while (Date.now() < deadline) {
    if (await isAcked(id)) return "acked";
    const row = await getOutboxEntry(id);
    if (!row) {
      // Not in store — treat as nothing to wait for.
      return "acked";
    }
    if (row.status === "failed_terminal") {
      const msg = row.lastError ?? "outbox failed_terminal";
      if (!opts?.silentToast) {
        toast.error("Cloud sync failed", msg);
      }
      if (opts?.throwOnTerminal) {
        throw new Error(msg);
      }
      return "failed_terminal";
    }
    await flushImOutbox();
    if (await isAcked(id)) return "acked";
    // Backoff head may not be due yet — yield briefly.
    await new Promise((r) => setTimeout(r, 40));
    scheduleOutboxWorker();
  }
  if (await isAcked(id)) return "acked";
  return "timeout";
}

/**
 * Uplink a Host-local agent final bubble to Hub (host_projection) via Outbox.
 *
 * Idempotent via `client_message_id` = local `agent-result:…` id.
 * Enqueue + flush (same status machine as user_message); never fire-and-forget.
 */
export async function syncAgentResultToCloud(input: {
  conversationId: string;
  messageId: string;
  text: string;
  agentRuntime?: string | null;
  agentSessionId?: string | null;
  replyToMessageId?: string | null;
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  const text = input.text.trim();
  const messageId = input.messageId.trim();
  const conversationId = input.conversationId.trim();
  if (!text || !messageId || !conversationId) return;
  // Uplink gate (C2): only canonical agent-result:{conv}:{session}:{origin}.
  if (!isCanonicalAgentResultId(messageId, conversationId)) {
    return;
  }
  if (!isProjectableAgentMessage({ id: messageId, role: "agent", body: text })) {
    return;
  }
  if ((await isAcked(messageId)) || projectedMessageIds.has(messageId)) return;

  const runtime =
    normalizeRuntime(input.agentRuntime) ??
    normalizeRuntime(
      RUNTIMES_FROM_ID.find((r) => messageId.toLowerCase().includes(r)) ?? null,
    );

  // Best-effort resolve at enqueue; worker re-resolves if missing.
  let agentId: string | null = null;
  if (runtime) {
    agentId = await resolveCloudAgentId(runtime);
  }

  const sessionId =
    input.agentSessionId?.trim() ||
    sessionIdFromAgentResultId(messageId) ||
    null;

  await enqueueAgentResult({
    conversationId,
    clientMessageId: messageId,
    text,
    agentId,
    agentSessionId: sessionId,
    agentRuntimes: runtime ? [runtime] : undefined,
    replyToMessageId: input.replyToMessageId ?? null,
  });

  // Lane worker only — never bypass FIFO with a hot-path flush.
  await waitForOutboxSettlement(messageId, { silentToast: false });
}

/**
 * After Linked timeline hydrate: project local agent-result rows that Hub is
 * missing (Desktop-native turns). Canonical id only — skip when Hub already
 * has the same message id (or outbox acked).
 */
export async function projectMissingLocalAgentResultsToHub(
  conversationId: string,
  localMessages: TimelineMessage[],
  hubMessages: TimelineMessage[],
): Promise<void> {
  if (!cloudAuth() || !conversationId.trim()) return;

  const hubIds = new Set(hubMessages.map((m) => m.id));

  for (const m of localMessages) {
    if (!isProjectableAgentMessage(m)) continue;
    // Only uplink frozen formula ids.
    if (!isCanonicalAgentResultId(m.id, conversationId)) continue;
    if ((await isAcked(m.id)) || projectedMessageIds.has(m.id)) continue;
    // Same canonical id already on Hub — no soft session/body dedupe (C2).
    if (hubIds.has(m.id)) {
      rememberProjected(m.id);
      continue;
    }

    const body = (m.body ?? "").trim();
    await syncAgentResultToCloud({
      conversationId,
      messageId: m.id,
      text: body,
      agentRuntime: m.agent,
      agentSessionId: m.sessionId ?? sessionIdFromAgentResultId(m.id),
      replyToMessageId: m.replyToMessageId ?? null,
    });
  }
}

/** Schedule a background outbox drain until no pending rows remain. */
export function scheduleOutboxWorker(): void {
  if (outboxWorkerTimer != null) return;
  outboxWorkerTimer = setTimeout(() => {
    outboxWorkerTimer = null;
    void (async () => {
      const now = Date.now();
      const dueNow = await listDuePending(now);
      const nextAt = await earliestPendingAttemptAt(now);
      if (dueNow.length === 0 && nextAt == null) return;
      if (dueNow.length === 0 && nextAt != null) {
        const delay = Math.max(50, Math.min(nextAt - Date.now(), 5 * 60_000));
        outboxWorkerTimer = setTimeout(() => {
          outboxWorkerTimer = null;
          void flushImOutbox().then(async () => {
            if ((await earliestPendingAttemptAt()) != null) {
              scheduleOutboxWorker();
            }
          });
        }, delay);
        return;
      }
      await flushImOutbox();
      if ((await earliestPendingAttemptAt()) != null) {
        scheduleOutboxWorker();
      }
    })();
  }, 50);
}

/** Start background outbox drain (call once when account session is ready). */
export function startImOutboxWorker(): void {
  void (async () => {
    await initImOutbox();
    await reclaimStaleInflight();
    await flushImOutbox();
    if ((await earliestPendingAttemptAt()) != null) {
      scheduleOutboxWorker();
    }
  })();
}

/** Clear process caches (tests / account switch). */
export function resetImCloudSyncStateForTests(): void {
  runtimeAgentIdCache.clear();
  projectedMessageIds.clear();
}
