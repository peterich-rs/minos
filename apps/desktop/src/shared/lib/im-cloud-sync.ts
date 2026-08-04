/**
 * Desktop → Hub IM helpers (conversation shell + user-message Outbox).
 *
 * Agent final bubbles:
 * - Hub TurnCompletionProjector covers Mobile `client_live` dispatches.
 * - Desktop-native turns never arm that watcher (user rows use host_projection).
 * - After local conversation_completion, Host uplinks agent-result via
 *   POST …/agents/message (host_projection + stable client_message_id) so
 *   Mobile/other devices see the final bubble (SSOT §2.4 Host Outbox path).
 *
 * User messages:
 * - Linked / authenticated: Hub-first (client_live) via Outbox (Phase 3.4)
 * - Desktop native path uses host_projection so Hub does not re-dispatch
 *
 * Conversation shell upsert + host-runtime agent resolve remain.
 */

import { useAccountStore } from "@/store/account-store";
import {
  addAgentToConversation,
  ensureHostRuntimeAgent,
  respondHubApproval,
  sendAgentConversationMessage,
  sendConversationMessage,
  toggleHubReaction,
  upsertConversation,
} from "@/shared/lib/minos-cloud";
import { isHubImMode } from "@/shared/lib/hub-timeline";
import {
  displayNameForRuntime,
  isCanonicalAgentResultId,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";
import { sessionIdFromAgentResultId } from "@/shared/lib/hub-timeline";
import {
  enqueueAgentResult,
  enqueueApprovalResolve,
  enqueueReactionToggle,
  enqueueUserMessage,
  isAcked,
  listDuePending,
  markAcked,
  markFailed,
  markInflight,
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

let outboxWorkerRunning = false;
let outboxWorkerTimer: ReturnType<typeof setTimeout> | null = null;

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
  markAcked(id);
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

async function postUserMessageFromOutbox(entry: ImOutboxEntry): Promise<void> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("Not signed in — message not synced to cloud");
  }

  if (entry.title?.trim() || (entry.agentRuntimes?.length ?? 0) > 0) {
    await syncConversationToCloud({
      conversationId: entry.conversationId,
      title: entry.title?.trim() || "Conversation",
      agentRuntimes: entry.agentRuntimes,
    });
  }

  // Linked Hub-first sends use client_live so Hub can @-dispatch.
  // Entries without explicit source default to client_live (Phase 3).
  const messageSource = entry.messageSource ?? "client_live";

  await sendConversationMessage(
    auth.deviceId,
    auth.accessToken,
    entry.conversationId,
    {
      text: entry.text,
      clientMessageId: entry.clientMessageId,
      replyToMessageId: entry.replyToMessageId ?? undefined,
      clientSentAtMs: entry.clientSentAtMs ?? undefined,
      messageSource,
    },
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
  return (
    code.includes("already_resolved") ||
    code.includes("approval_already") ||
    code.includes("approval_not_found") ||
    msg.includes("already_resolved") ||
    msg.includes("already resolved") ||
    msg.includes("approval_already") ||
    msg.includes("not found") // request gone after host applied
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
      isHubImMode({
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

async function postOutboxEntry(entry: ImOutboxEntry): Promise<void> {
  switch (entry.kind) {
    case "user_message":
      await postUserMessageFromOutbox(entry);
      return;
    case "agent_result":
      await postAgentResultFromOutbox(entry);
      return;
    case "reaction_toggle":
      await postReactionToggleFromOutbox(entry);
      return;
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
 * C5.1 single path: enqueue reaction_toggle then flush via the same outbox
 * machine as user_message. Returns Hub aggregate for generation-gated apply.
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
  if (isAcked(clientOpId)) {
    return null;
  }
  if (!cloudAuth()) {
    throw new Error("not authenticated");
  }

  const entry = enqueueReactionToggle({
    conversationId,
    clientMessageId: clientOpId,
    text: JSON.stringify({ messageId, emoji }),
  });
  if (entry.status === "acked" || isAcked(clientOpId)) {
    return null;
  }

  markInflight(clientOpId);
  try {
    const result = await postReactionToggleFromOutbox(entry);
    markAcked(clientOpId);
    return result;
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    markFailed(clientOpId, msg);
    scheduleOutboxWorker();
    throw error instanceof Error ? error : new Error(msg);
  }
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
  if (isAcked(clientOpId)) return;

  const decision = { ...input.decision };
  delete decision.client_request_id;

  const { session, authPhase } = useAccountStore.getState();
  const route =
    input.route ??
    (isHubImMode({
      authPhase,
      accessToken: session?.accessToken,
    })
      ? "hub"
      : "daemon");

  if (route === "hub" && !cloudAuth()) {
    throw new Error("not authenticated");
  }

  const entry = enqueueApprovalResolve({
    conversationId: sessionId,
    clientMessageId: clientOpId,
    text: JSON.stringify({
      requestId,
      sessionId,
      decision,
      route,
    }),
  });
  if (entry.status === "acked" || isAcked(clientOpId)) return;

  markInflight(clientOpId);
  try {
    await postApprovalResolveFromOutbox(entry);
    markAcked(clientOpId);
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    markFailed(clientOpId, msg);
    scheduleOutboxWorker();
    throw error instanceof Error ? error : new Error(msg);
  }
}

/**
 * Enqueue + flush a user timeline message to hub.
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
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  const text = input.text.trim();
  const messageId = input.messageId.trim();
  if (!text || !input.conversationId.trim() || !messageId) return;
  if (isAcked(messageId)) return;

  const entry = enqueueUserMessage({
    conversationId: input.conversationId,
    clientMessageId: messageId,
    text,
    title: input.title,
    replyToMessageId: input.replyToMessageId,
    agentRuntimes: input.agentRuntimes,
    clientSentAtMs: input.createdAtMs,
    messageSource: input.messageSource ?? "client_live",
  });

  await flushOutboxEntry(entry, { throwOnTerminal: true });
  scheduleOutboxWorker();
}

async function flushOutboxEntry(
  entry: ImOutboxEntry,
  opts?: { throwOnTerminal?: boolean },
): Promise<void> {
  if (entry.status === "acked") return;
  if (isAcked(entry.clientMessageId)) return;

  markInflight(entry.clientMessageId);
  try {
    await postOutboxEntry(entry);
    markAcked(entry.clientMessageId);
    rememberProjected(entry.clientMessageId);
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    const status = markFailed(entry.clientMessageId, msg);
    console.warn("[im-cloud-sync] outbox flush failed", entry.kind, error);
    if (status === "failed_terminal") {
      toast.error(
        "Cloud sync failed",
        `Message not visible on other devices: ${msg}`,
      );
      if (opts?.throwOnTerminal) {
        throw error instanceof Error ? error : new Error(msg);
      }
    } else {
      toast.warning(
        "Cloud sync delayed",
        "Will retry sending this message to other devices.",
      );
      // Hub-first Linked: first attempt failure still surfaces as send error so
      // the optimistic row can flip to failed (outbox will retry in background).
      if (opts?.throwOnTerminal) {
        throw error instanceof Error ? error : new Error(msg);
      }
    }
  }
}

/** Drain due pending outbox rows (bounded concurrency 1). */
export async function flushImOutbox(): Promise<void> {
  if (outboxWorkerRunning) return;
  outboxWorkerRunning = true;
  try {
    const due = listDuePending();
    for (const entry of due) {
      await flushOutboxEntry(entry);
    }
  } finally {
    outboxWorkerRunning = false;
  }
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
  if (isAcked(messageId) || projectedMessageIds.has(messageId)) return;

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

  const entry = enqueueAgentResult({
    conversationId,
    clientMessageId: messageId,
    text,
    agentId,
    agentSessionId: sessionId,
    agentRuntimes: runtime ? [runtime] : undefined,
    replyToMessageId: input.replyToMessageId ?? null,
  });

  await flushOutboxEntry(entry);
  scheduleOutboxWorker();
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
    if (isAcked(m.id) || projectedMessageIds.has(m.id)) continue;
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

/** Schedule a background outbox drain (deduped; used after enqueue). */
export function scheduleOutboxWorker(): void {
  if (outboxWorkerTimer != null) return;
  outboxWorkerTimer = setTimeout(() => {
    outboxWorkerTimer = null;
    void flushImOutbox().then(() => {
      const still = listDuePending(Date.now() + 60_000);
      if (still.length > 0 || listDuePending().length > 0) {
        scheduleOutboxWorker();
      }
    });
  }, 2_000);
}

/** Start background outbox drain (call once when account session is ready). */
export function startImOutboxWorker(): void {
  reclaimStaleInflight();
  void flushImOutbox();
  scheduleOutboxWorker();
}

/** Clear process caches (tests / account switch). */
export function resetImCloudSyncStateForTests(): void {
  runtimeAgentIdCache.clear();
  projectedMessageIds.clear();
}
