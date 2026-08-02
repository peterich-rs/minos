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
  sendAgentConversationMessage,
  sendConversationMessage,
  upsertConversation,
} from "@/shared/lib/minos-cloud";
import {
  displayNameForRuntime,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";
import {
  agentResultSessionKey,
  sessionIdFromAgentResultId,
} from "@/shared/lib/hub-timeline";
import {
  enqueueUserMessage,
  isAcked,
  listDuePending,
  markAcked,
  markFailed,
  markInflight,
  type ImOutboxEntry,
} from "@/shared/lib/im-outbox";
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
    await postUserMessageFromOutbox(entry);
    markAcked(entry.clientMessageId);
    rememberProjected(entry.clientMessageId);
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    const status = markFailed(entry.clientMessageId, msg);
    console.warn("[im-cloud-sync] send user message failed", error);
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
 * Uplink a Host-local agent final bubble to Hub (host_projection).
 *
 * Idempotent via `client_message_id` = local `agent-result:…` id.
 * Skip when Hub already has any agent-result for the same session (Mobile
 * TurnCompletionProjector path uses a different durable suffix).
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
  if (!isProjectableAgentMessage({ id: messageId, role: "agent", body: text })) {
    return;
  }
  if (isAcked(messageId) || projectedMessageIds.has(messageId)) return;

  const runtime =
    normalizeRuntime(input.agentRuntime) ??
    // Infer from common display names when runtime missing on row.
    normalizeRuntime(
      RUNTIMES_FROM_ID.find((r) => messageId.toLowerCase().includes(r)) ?? null,
    );
  if (!runtime) {
    console.warn(
      "[im-cloud-sync] agent result uplink skipped: unknown runtime",
      messageId,
    );
    return;
  }

  const agentId = await resolveCloudAgentId(runtime);
  if (!agentId) {
    console.warn(
      "[im-cloud-sync] agent result uplink skipped: no cloud agent id",
      runtime,
    );
    return;
  }

  const sessionId =
    input.agentSessionId?.trim() ||
    sessionIdFromAgentResultId(messageId) ||
    undefined;

  try {
    await sendAgentConversationMessage(
      auth.deviceId,
      auth.accessToken,
      conversationId,
      {
        agentId,
        text,
        clientMessageId: messageId,
        agentSessionId: sessionId,
        replyToMessageId: input.replyToMessageId ?? undefined,
        // Never re-dispatch: Host already executed this turn.
        messageSource: "host_projection",
      },
    );
    rememberProjected(messageId);
    markAcked(messageId);
  } catch (error) {
    console.warn("[im-cloud-sync] agent result uplink failed", messageId, error);
  }
}

const RUNTIMES_FROM_ID = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
] as const;

/**
 * After Linked timeline hydrate: project local agent-result rows that Hub is
 * missing (Desktop-native turns). Skip when Hub already covers the session.
 */
export async function projectMissingLocalAgentResultsToHub(
  conversationId: string,
  localMessages: TimelineMessage[],
  hubMessages: TimelineMessage[],
): Promise<void> {
  if (!cloudAuth() || !conversationId.trim()) return;

  const hubSessions = new Set<string>();
  for (const m of hubMessages) {
    if (m.role !== "agent" && !m.id.startsWith("agent-result:")) continue;
    const key = agentResultSessionKey(m.id);
    if (key) hubSessions.add(key);
    if (m.sessionId?.trim()) hubSessions.add(`*:${m.sessionId.trim()}`);
  }

  for (const m of localMessages) {
    if (!isProjectableAgentMessage(m)) continue;
    if (isAcked(m.id) || projectedMessageIds.has(m.id)) continue;

    const sessionKey = agentResultSessionKey(m.id);
    if (sessionKey && hubSessions.has(sessionKey)) continue;
    if (m.sessionId?.trim() && hubSessions.has(`*:${m.sessionId.trim()}`)) {
      continue;
    }

    // Soft: Hub already has same body from projector (different id).
    const body = (m.body ?? "").trim();
    if (
      body &&
      hubMessages.some(
        (h) =>
          (h.role === "agent" || h.id.startsWith("agent-result:")) &&
          (h.body ?? "").trim() === body,
      )
    ) {
      rememberProjected(m.id);
      continue;
    }

    await syncAgentResultToCloud({
      conversationId,
      messageId: m.id,
      text: body,
      agentRuntime: m.agent,
      agentSessionId: m.sessionId ?? sessionIdFromAgentResultId(m.id),
      // Do not invent reply_to: Mobile projector owns reply causality for
      // client_live dispatches; Desktop-native turns stay unquoted.
      replyToMessageId: m.replyToMessageId ?? null,
    });
  }
}

function scheduleOutboxWorker(): void {
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
  void flushImOutbox();
  scheduleOutboxWorker();
}

/** Clear process caches (tests / account switch). */
export function resetImCloudSyncStateForTests(): void {
  runtimeAgentIdCache.clear();
  projectedMessageIds.clear();
}
