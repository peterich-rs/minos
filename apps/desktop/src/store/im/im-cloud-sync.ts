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

import { getCloudAuth } from "@/shared/lib/cloud-auth";
import {
  getAccountScopeGeneration,
} from "@/shared/lib/account-scope-generation";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import {
  isCanonicalAgentResultId,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";

import {
  clearRuntimeAgentIdCache,
  resolveCloudAgentId,
  RUNTIMES_FROM_ID,
} from "@/store/im/im-cloud-agents";
import {
  postOutboxEntry,
  type CloudUserMessageAck,
  type CloudReactionToggleResult,
  type OutboxScope,
} from "@/store/im/im-cloud-outbox-posts";
export type { CloudUserMessageAck, CloudReactionToggleResult } from "@/store/im/im-cloud-outbox-posts";
export {
  resolveCloudAgentId,
  resolveCloudAgentIds,
  syncConversationToCloud,
  attachAgentsToConversationCloud,
} from "@/store/im/im-cloud-agents";

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
import { toast } from "@/shared/lib/toast";
import type { TimelineMessage } from "@/shared/domain/collaboration";

export {
  displayNameForRuntime,
  isProjectableAgentMessage,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";

/** In-memory runtime → cloud agent_id for this process. */
/** Track acked / seen hub message ids (user outbox + inbound echo). */
const projectedMessageIds = new Set<string>();
const MAX_PROJECTED_CACHE = 4000;

/** Per-lane drain chains — different conversations / intent classes parallelize. */
const laneChains = new Map<string, Promise<void>>();
let outboxWorkerTimer: ReturnType<typeof setTimeout> | null = null;
/** Bumped on account leave so scheduled drains cannot flush under a new session. */
let outboxWorkerGeneration = 0;

/** Last successful reaction toggle payload by client op id (for waiters). */
const userMessageAcks = new Map<string, CloudUserMessageAck>();

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

function cloudAuth(): {
  deviceId: string;
  accessToken: string;
  accountId: string;
} | null {
  const auth = getCloudAuth();
  if (!auth) return null;
  const accessToken = auth.accessToken.trim();
  const accountId = auth.accountId.trim();
  if (!accessToken || !accountId) return null;
  return {
    deviceId: auth.deviceId,
    accessToken,
    accountId,
  };
}

function currentOutboxAccountId(): string {
  return cloudAuth()?.accountId ?? "";
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
export async function syncReactionToggleToCloud(input: {
  conversationId: string;
  messageId: string;
  emoji: string;
  clientOpId: string;
}): Promise<CloudReactionToggleResult | null> {
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
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("not authenticated");
  }
  const entry = await enqueueReactionToggle({
    conversationId,
    clientMessageId: clientOpId,
    accountId: auth.accountId,
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
 * Durable approval intent via Intent Outbox.
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

  const authSnap = getCloudAuth();
  const route =
    input.route ??
    (isCloudImMode({
      authPhase: authSnap?.authPhase,
      accessToken: authSnap?.accessToken,
    })
      ? "hub"
      : "daemon");

  const auth = cloudAuth();
  if (route === "hub" && !auth) {
    throw new Error("not authenticated");
  }
  const accountId = auth?.accountId ?? authSnap?.accountId?.trim() ?? "";
  if (!accountId) {
    throw new Error("not authenticated");
  }

  const entry = await enqueueApprovalResolve({
    // Scope key for local storage only (not a Hub conversation id).
    conversationId: `approval-session:${sessionId}`,
    clientMessageId: clientOpId,
    accountId,
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

export { deliveryStatusAfterUserSettlement } from "@/shared/lib/user-message-settlement";

/**
 * Enqueue a user timeline message then drain via per-conversation message lane.
 * Surfaces toast on terminal failure (multi-end intent must not silent-succeed).
 * Resolves only when outbox is acked — never treats timeout as success.
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
}): Promise<CloudUserMessageAck | null> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("not authenticated");
  }
  const text = input.text.trim();
  const messageId = input.messageId.trim();
  if (!text || !input.conversationId.trim() || !messageId) {
    throw new Error("invalid user message for cloud sync");
  }
  if (await isAcked(messageId)) {
    return userMessageAcks.get(messageId) ?? null;
  }

  await enqueueUserMessage({
    conversationId: input.conversationId,
    clientMessageId: messageId,
    accountId: auth.accountId,
    text,
    title: input.title,
    replyToMessageId: input.replyToMessageId,
    agentRuntimes: input.agentRuntimes,
    clientSentAtMs: input.createdAtMs,
    messageSource: input.messageSource ?? "client_live",
    mentions: input.mentions,
  });

  const settlement = await waitForOutboxSettlement(messageId, {
    throwOnTerminal: true,
  });
  if (settlement !== "acked") {
    // timeout (failed_terminal already throws when throwOnTerminal)
    throw new Error(
      "Cloud sync timed out — message still pending delivery to other devices",
    );
  }
  // Acked may lack ChatSendAck payload (seq) — still durable success.
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
  const auth = cloudAuth();
  if (!auth || entry.accountId.trim() !== auth.accountId) {
    // Never send under a different (or missing) account identity.
    return;
  }
  // Capture scope at lane start; re-validate after every await / before Hub write.
  const scope: OutboxScope = {
    accountId: auth.accountId,
    scopeGeneration: getAccountScopeGeneration(),
  };

  await markInflight(entry.clientMessageId);
  if (
    getAccountScopeGeneration() !== scope.scopeGeneration ||
    cloudAuth()?.accountId !== scope.accountId
  ) {
    // Leave mid-flight: do not post under the new account. Reclaim via stale TTL.
    return;
  }
  try {
    const result = await postOutboxEntry(entry, scope);
    if (
      getAccountScopeGeneration() !== scope.scopeGeneration ||
      cloudAuth()?.accountId !== scope.accountId
    ) {
      // Sent under race — do not mark acked for the wrong scope; reclaim later.
      return;
    }
    await markAcked(entry.clientMessageId);
    if (entry.kind === "user_message" || entry.kind === "agent_result") {
      rememberProjected(entry.clientMessageId);
    }
    if (entry.kind === "reaction_toggle" && result) {
      reactionResults.set(
        entry.clientMessageId,
        result as CloudReactionToggleResult,
      );
    }
    if (entry.kind === "user_message" && result && "messageSeq" in result) {
      userMessageAcks.set(entry.clientMessageId, result as CloudUserMessageAck);
    }
    // Waiter may have already timed out and marked the bubble failed. When the
    // durable outbox later acks, upgrade UI so a background success is visible
    // without requiring a manual retry.
    if (entry.kind === "user_message") {
      const ack =
        result && "messageSeq" in result
          ? (result as CloudUserMessageAck)
          : userMessageAcks.get(entry.clientMessageId);
      try {
        const { patchMessageDelivery } = await import(
          "@/store/workspace/timeline-write"
        );
        const row = (
          await import("@/store/workspace-store")
        ).useWorkspaceStore.getState().messagesByConversation[
          entry.conversationId
        ]?.find((m) => m.id === entry.clientMessageId);
        if (
          row &&
          (row.deliveryStatus === "sending" || row.deliveryStatus === "failed")
        ) {
          patchMessageDelivery(entry.conversationId, entry.clientMessageId, {
            deliveryStatus: "sent",
            ...(ack?.messageSeq != null ? { messageSeq: ack.messageSeq } : {}),
          });
        }
      } catch {
        /* timeline store optional in pure unit tests */
      }
    }
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    // Scope abort is not a delivery failure — leave inflight for reclaim.
    if (msg.includes("account scope changed")) {
      return;
    }
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
    const accountId = currentOutboxAccountId();
    if (!accountId) return;
    // Re-list so we only take this lane's current due head-run.
    const lanes = await listDuePendingLanes(Date.now(), accountId);
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
  const accountId = currentOutboxAccountId();
  if (!accountId) return;
  const lanes = await listDuePendingLanes(Date.now(), accountId);
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
  if (opts?.throwOnTerminal) {
    throw new Error("outbox settlement timeout");
  }
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
  // Uplink gate: only canonical agent-result:{conv}:{session}:{origin}.
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
    accountId: auth.accountId,
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
export async function projectMissingLocalAgentResultsToCloud(
  conversationId: string,
  localMessages: TimelineMessage[],
  cloudMessages: TimelineMessage[],
): Promise<void> {
  if (!cloudAuth() || !conversationId.trim()) return;

  const cloudIds = new Set(cloudMessages.map((m) => m.id));

  for (const m of localMessages) {
    if (!isProjectableAgentMessage(m)) continue;
    // Only uplink frozen formula ids.
    if (!isCanonicalAgentResultId(m.id, conversationId)) continue;
    if ((await isAcked(m.id)) || projectedMessageIds.has(m.id)) continue;
    // Same canonical id already on Hub — no soft session/body dedupe.
    if (cloudIds.has(m.id)) {
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
  const gen = outboxWorkerGeneration;
  outboxWorkerTimer = setTimeout(() => {
    outboxWorkerTimer = null;
    if (gen !== outboxWorkerGeneration) return;
    void (async () => {
      if (gen !== outboxWorkerGeneration) return;
      const accountId = currentOutboxAccountId();
      if (!accountId) return;
      const now = Date.now();
      const dueNow = await listDuePending(now, accountId);
      const nextAt = await earliestPendingAttemptAt(now, accountId);
      if (dueNow.length === 0 && nextAt == null) return;
      if (gen !== outboxWorkerGeneration) return;
      if (dueNow.length === 0 && nextAt != null) {
        const delay = Math.max(50, Math.min(nextAt - Date.now(), 5 * 60_000));
        if (outboxWorkerTimer != null) return;
        outboxWorkerTimer = setTimeout(() => {
          outboxWorkerTimer = null;
          if (gen !== outboxWorkerGeneration) return;
          void flushImOutbox().then(async () => {
            if (gen !== outboxWorkerGeneration) return;
            if (
              (await earliestPendingAttemptAt(
                Date.now(),
                currentOutboxAccountId(),
              )) != null
            ) {
              scheduleOutboxWorker();
            }
          });
        }, delay);
        return;
      }
      await flushImOutbox();
      if (gen !== outboxWorkerGeneration) return;
      if (
        (await earliestPendingAttemptAt(
          Date.now(),
          currentOutboxAccountId(),
        )) != null
      ) {
        scheduleOutboxWorker();
      }
    })();
  }, 50);
}

/** Stop background outbox drain (account leave). */
export function stopImOutboxWorker(): void {
  outboxWorkerGeneration += 1;
  if (outboxWorkerTimer != null) {
    clearTimeout(outboxWorkerTimer);
    outboxWorkerTimer = null;
  }
}

/** Start background outbox drain (call once when account session is ready). */
export function startImOutboxWorker(): void {
  const gen = outboxWorkerGeneration;
  void (async () => {
    await initImOutbox();
    if (gen !== outboxWorkerGeneration) return;
    const accountId = currentOutboxAccountId();
    await reclaimStaleInflight(Date.now(), accountId);
    if (gen !== outboxWorkerGeneration) return;
    await flushImOutbox();
    if (gen !== outboxWorkerGeneration) return;
    if (
      (await earliestPendingAttemptAt(
        Date.now(),
        currentOutboxAccountId(),
      )) != null
    ) {
      scheduleOutboxWorker();
    }
  })();
}

/** Clear process caches (account leave / tests). */
export function resetImCloudSyncState(): void {
  stopImOutboxWorker();
  clearRuntimeAgentIdCache();
  projectedMessageIds.clear();
  reactionResults.clear();
  userMessageAcks.clear();
  laneChains.clear();
}
