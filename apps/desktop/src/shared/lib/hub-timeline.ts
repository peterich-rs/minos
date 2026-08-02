/**
 * Hub conversation messages → Desktop TimelineMessage projection (Phase 3).
 *
 * Linked mode: Hub bubbles are the primary chat SSOT projection.
 * Local daemon tool/git/system cards are merged separately by the timeline loader.
 */

import type { AgentRuntime, TimelineMessage } from "./mock-data.ts";
import type { HubChatMessage } from "./minos-cloud.ts";
import { formatLocalClock } from "./time.ts";
import { normalizeHostRuntime } from "./im-cloud-sync-helpers.ts";
import { sortTimelineMessages } from "./timeline-order.ts";

const RUNTIMES: AgentRuntime[] = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
];

/** True when Minos account is authenticated (multi-end IM intent). */
export function isHubImMode(input: {
  authPhase?: string | null;
  accessToken?: string | null;
}): boolean {
  return (
    input.authPhase === "authenticated" &&
    Boolean(input.accessToken?.trim())
  );
}

/** Infer runtime bin from hub agent display / id map. */
export function runtimeFromHubAgent(message: HubChatMessage): AgentRuntime | undefined {
  if (message.senderType !== "agent") return undefined;
  const name = `${message.senderDisplayName} ${message.senderMinosId}`.toLowerCase();
  for (const r of RUNTIMES) {
    if (name.includes(r)) return r;
  }
  return undefined;
}

export function hubChatMessageToTimeline(
  message: HubChatMessage,
  opts?: { agentRuntimeMap?: Map<string, string> },
): TimelineMessage | null {
  if (!message.messageId) return null;
  // Recalled: callers use removeMessageFromTimeline; do not re-insert.
  if (message.recalledAtMs) {
    return null;
  }
  const text = message.text?.trim();
  if (!text) return null;

  let agent: AgentRuntime | undefined;
  if (message.senderType === "agent") {
    const mapped = opts?.agentRuntimeMap?.get(message.senderAccountId);
    const fromMap = mapped ? normalizeHostRuntime(mapped) : null;
    agent = (fromMap as AgentRuntime | null) ?? runtimeFromHubAgent(message);
  }

  return {
    id: message.messageId,
    role: message.senderType === "agent" ? "agent" : "user",
    agent,
    body: text,
    time: formatLocalClock(message.createdAtMs),
    createdAtMs: message.createdAtMs,
    kind: "text",
    replyToMessageId: message.replyToMessageId ?? undefined,
    deliveryStatus: "sent",
  };
}

/** Remove a hub message id from the window (recall / SnapshotRequired rebuild). */
export function removeMessageFromTimeline(
  prev: TimelineMessage[] | undefined,
  messageId: string,
): TimelineMessage[] {
  if (!prev?.length || !messageId) return prev ?? [];
  const next = prev.filter((m) => m.id !== messageId);
  return next.length === prev.length ? prev : next;
}

/**
 * Whether a local daemon timeline row is a multi-end chat bubble that Hub owns
 * when Linked. Tool/git/system/approval cards stay local.
 */
export function isLocalChatBubbleForHubSsot(m: TimelineMessage): boolean {
  const kind = m.kind ?? "text";
  if (kind === "tool_summary" || kind === "git_activity" || kind === "approval") {
    return false;
  }
  if (kind === "system" && m.role === "system") {
    return false;
  }
  // User / agent text (incl. agent-result:…) — Hub SSOT when Linked.
  if (m.role === "user" || m.role === "agent") {
    return true;
  }
  if (m.id.startsWith("agent-result:")) {
    return true;
  }
  return false;
}

/**
 * Session correlation key for `agent-result:{conversationId}:{sessionId}:{durable}`.
 *
 * Hub TurnCompletionProjector uses `…:{trigger_seq}` while daemon
 * conversation_completion uses `…:{message_key|t{ms}}` — different durable
 * suffixes for the **same turn**. Merging without this key produces duplicate
 * Grok rows (one plain + one with reply-to preview).
 */
export function agentResultSessionKey(messageId: string): string | null {
  if (!messageId.startsWith("agent-result:")) return null;
  const rest = messageId.slice("agent-result:".length);
  const first = rest.indexOf(":");
  if (first <= 0) return null;
  const second = rest.indexOf(":", first + 1);
  if (second <= first + 1) return null;
  // conversationId:sessionId (ignore durable suffix)
  return rest.slice(0, second);
}

/** Parse session id from agent-result message id when present. */
export function sessionIdFromAgentResultId(messageId: string): string | null {
  const key = agentResultSessionKey(messageId);
  if (!key) return null;
  const idx = key.indexOf(":");
  if (idx < 0) return null;
  const sessionId = key.slice(idx + 1).trim();
  return sessionId || null;
}

function isAgentChatBubble(m: TimelineMessage): boolean {
  return m.role === "agent" || m.id.startsWith("agent-result:");
}

/**
 * Merge Hub chat bubbles with local non-bubble cards (tool/git/system).
 *
 * Rules:
 * - Hub wins on **same message id** for chat bubbles (multi-end SSOT).
 * - Local tool/git/system/approval cards always keep.
 * - Local chat bubbles **missing from Hub** are gap-filled:
 *   Desktop native agent-result rows, optimistic user sends, and any
 *   host-local user rows not yet projected — otherwise Linked merge
 *   would hide agent replies while the session inspector shows Idle+reply.
 * - Local `agent-result:…` is **not** gap-filled when Hub already has any
 *   agent-result for the same conversation+session (id durable suffix differs).
 */
export function mergeHubAndLocalTimeline(input: {
  hubMessages: TimelineMessage[];
  localMessages: TimelineMessage[];
}): TimelineMessage[] {
  const byId = new Map<string, TimelineMessage>();

  // Local non-chat cards first (gaps in timeline).
  for (const m of input.localMessages) {
    if (isLocalChatBubbleForHubSsot(m)) continue;
    byId.set(m.id, m);
  }

  // Hub session keys already represented by multi-end agent bubbles.
  const hubAgentSessions = new Set<string>();
  for (const m of input.hubMessages) {
    if (!isAgentChatBubble(m)) continue;
    const key = agentResultSessionKey(m.id);
    if (key) hubAgentSessions.add(key);
    // Also correlate by explicit sessionId when id is not agent-result-shaped.
    if (m.sessionId?.trim()) {
      // Use empty conversation prefix only when id lacks agent-result shape;
      // prefer full agent-result key when available.
      if (!key) {
        hubAgentSessions.add(`*:${m.sessionId.trim()}`);
      }
    }
  }

  // Hub bubbles are authoritative when present; keep local messageSeq so sort
  // still works (Hub HTTP rows often omit host-local seq).
  for (const m of input.hubMessages) {
    const localPeer = input.localMessages.find((l) => l.id === m.id);
    if (localPeer?.messageSeq != null && m.messageSeq == null) {
      byId.set(m.id, { ...m, messageSeq: localPeer.messageSeq });
    } else {
      byId.set(m.id, m);
    }
  }

  // Gap-fill local chat not yet on Hub (same id → already covered above).
  for (const m of input.localMessages) {
    if (!isLocalChatBubbleForHubSsot(m)) continue;
    if (byId.has(m.id)) {
      // Same id already from Hub: ensure seq retained if Hub lacked it.
      const cur = byId.get(m.id)!;
      if (cur.messageSeq == null && m.messageSeq != null) {
        byId.set(m.id, { ...cur, messageSeq: m.messageSeq });
      }
      continue;
    }

    // Suppress local agent-result siblings when Hub already has the turn.
    if (isAgentChatBubble(m)) {
      const sessionKey = agentResultSessionKey(m.id);
      if (sessionKey && hubAgentSessions.has(sessionKey)) {
        continue;
      }
      if (m.sessionId?.trim() && hubAgentSessions.has(`*:${m.sessionId.trim()}`)) {
        continue;
      }
      // Body+time soft dedupe: Hub agent with same text within 2 minutes.
      const body = (m.body ?? "").trim();
      if (body) {
        const localMs = m.createdAtMs ?? 0;
        const hubDup = input.hubMessages.some((h) => {
          if (!isAgentChatBubble(h)) return false;
          if ((h.body ?? "").trim() !== body) return false;
          if (!localMs || !h.createdAtMs) return true;
          return Math.abs(h.createdAtMs - localMs) <= 120_000;
        });
        if (hubDup) continue;
      }
    }

    // User optimistic / not-yet-synced; agent-result from daemon completion
    // when Hub projector has not posted yet (Desktop-native turns).
    if (
      m.role === "user" ||
      m.role === "agent" ||
      m.id.startsWith("agent-result:")
    ) {
      byId.set(m.id, m);
    }
  }

  return sortTimelineMessages([...byId.values()]);
}

/** Apply one hub realtime message into an existing timeline window. */
export function upsertHubMessageIntoTimeline(
  prev: TimelineMessage[] | undefined,
  hub: TimelineMessage,
): TimelineMessage[] {
  const list = prev ?? [];
  const byId = new Map(list.map((m) => [m.id, m]));
  const existing = byId.get(hub.id);
  if (
    existing &&
    existing.body === hub.body &&
    existing.createdAtMs === hub.createdAtMs &&
    existing.role === hub.role &&
    existing.replyToMessageId === hub.replyToMessageId
  ) {
    return list;
  }

  // Hub agent-result for a session supersedes local siblings (different durable id).
  if (isAgentChatBubble(hub)) {
    const hubSession = agentResultSessionKey(hub.id);
    const hubSessionId = hub.sessionId?.trim() || sessionIdFromAgentResultId(hub.id);
    for (const [id, m] of [...byId.entries()]) {
      if (id === hub.id) continue;
      if (!isAgentChatBubble(m)) continue;
      const localSession = agentResultSessionKey(id);
      if (hubSession && localSession && hubSession === localSession) {
        byId.delete(id);
        continue;
      }
      if (
        hubSessionId &&
        (m.sessionId?.trim() === hubSessionId ||
          sessionIdFromAgentResultId(id) === hubSessionId)
      ) {
        // Prefer Hub row; drop local agent-result for same session.
        if (id.startsWith("agent-result:") && id !== hub.id) {
          byId.delete(id);
        }
      }
    }
  }

  byId.set(hub.id, hub);
  return sortTimelineMessages([...byId.values()]);
}
