/**
 * Hub conversation messages → Desktop TimelineMessage projection.
 *
 * Linked mode: Hub bubbles are the primary chat SSOT projection.
 * Local daemon tool/git/system cards are merged separately by the timeline loader.
 *
 * Agent final bubble id (frozen, IM reliability):
 *   agent-result:{conversationId}:{sessionId}:{originMessageId}
 * Merge is by **message id equality** only — no body/time soft-dedupe, no
 * fuzzy `*:sessionId` keys.
 */

import type { AgentRuntime, TimelineMessage } from "./mock-data.ts";
import type { HubChatMessage } from "./minos-cloud.ts";
import { formatLocalClock } from "./time.ts";
import { normalizeHostRuntime } from "./im-cloud-sync-helpers.ts";
import {
  isHostOnlyTimelineCard,
  sortTimelineMessages,
} from "./timeline-order.ts";

const RUNTIMES: AgentRuntime[] = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
];

/** True when Minos account is authenticated (multi-end IM intent). */
export function isCloudImMode(input: {
  authPhase?: string | null;
  accessToken?: string | null;
}): boolean {
  return (
    input.authPhase === "authenticated" &&
    Boolean(input.accessToken?.trim())
  );
}

/** Infer runtime bin from hub agent display / id map (fallback only). */
export function runtimeFromCloudAgent(message: HubChatMessage): AgentRuntime | undefined {
  if (message.senderType !== "agent") return undefined;
  // Prefer wire MessageSender.runtime_agent (badge field, not identity).
  if (message.runtimeAgent) {
    const fromWire = normalizeHostRuntime(message.runtimeAgent);
    if (fromWire && (RUNTIMES as string[]).includes(fromWire)) {
      return fromWire as AgentRuntime;
    }
  }
  const name = `${message.senderDisplayName} ${message.senderMinosId}`.toLowerCase();
  for (const r of RUNTIMES) {
    if (name.includes(r)) return r;
  }
  return undefined;
}

export function cloudChatMessageToTimeline(
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

  const isAgent = message.senderType === "agent";
  let agent: AgentRuntime | undefined;
  if (isAgent) {
    const mapped = opts?.agentRuntimeMap?.get(message.senderAccountId);
    const fromMap = mapped ? normalizeHostRuntime(mapped) : null;
    agent = (fromMap as AgentRuntime | null) ?? runtimeFromCloudAgent(message);
  }

  // Preserve explicit empty array so merge treats Hub "no reactions" as SSOT
  // (do not collapse [] → undefined and fall back to stale local).
  const reactions =
    message.reactions === undefined
      ? undefined
      : message.reactions.map((g) => ({
          emoji: g.emoji,
          count: g.count,
          reactedByMe: g.reactedByMe,
          actors: (g.actors ?? []).map((a) => ({
            id: a.actorId,
            displayName: a.displayName,
          })),
        }));

  const mentions = [
    ...(message.mentionedAccountIds ?? []).map((id) => ({
      kind: "account" as const,
      targetId: id,
    })),
    ...(message.mentionedAgentIds ?? []).map((id) => {
      const runtime = opts?.agentRuntimeMap?.get(id);
      return {
        kind: "agent" as const,
        targetId: id,
        agent: runtime,
      };
    }),
  ];

  const displayName = message.senderDisplayName?.trim() || undefined;

  return {
    id: message.messageId,
    role: isAgent ? "agent" : "user",
    agent,
    // Bot identity presentation: display name + global bot id from MessageSender.
    senderDisplayName: displayName,
    botId: isAgent ? message.senderAccountId : undefined,
    body: text,
    time: formatLocalClock(message.createdAtMs),
    createdAtMs: message.createdAtMs,
    // Missing message_seq stays undefined — never coerce to 0.
    messageSeq:
      message.messageSeq != null && Number.isFinite(message.messageSeq)
        ? message.messageSeq
        : undefined,
    kind: "text",
    replyToMessageId: message.replyToMessageId ?? undefined,
    deliveryStatus: "sent",
    reactions,
    mentions: mentions.length > 0 ? mentions : undefined,
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
export function isLocalChatBubbleForCloudSsot(m: TimelineMessage): boolean {
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
 * Session correlation key for `agent-result:{conversationId}:{sessionId}:{origin}`.
 * Used for diagnostics / projection helpers — **not** for soft-dedupe merge.
 */
export function agentResultSessionKey(messageId: string): string | null {
  if (!messageId.startsWith("agent-result:")) return null;
  const rest = messageId.slice("agent-result:".length);
  const first = rest.indexOf(":");
  if (first <= 0) return null;
  const second = rest.indexOf(":", first + 1);
  if (second <= first + 1) return null;
  // conversationId:sessionId (ignore origin suffix)
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

function isOptimisticLocalBubble(m: TimelineMessage): boolean {
  return m.deliveryStatus === "sending" || m.deliveryStatus === "failed";
}

/**
 * Pick Hub message_seq to hang a host card after (anchor).
 * Prefer last Hub bubble with createdAtMs ≤ card time among loaded hub rows.
 */
function anchorForHostCard(
  card: TimelineMessage,
  hubBubbles: TimelineMessage[],
): number | undefined {
  let best: number | undefined;
  let bestTs = -Infinity;
  const cardTs = card.createdAtMs ?? Number.POSITIVE_INFINITY;
  for (const h of hubBubbles) {
    if (h.messageSeq == null || !Number.isFinite(h.messageSeq)) continue;
    const ts = h.createdAtMs ?? 0;
    if (ts <= cardTs && ts >= bestTs) {
      bestTs = ts;
      best = h.messageSeq;
    }
  }
  return best;
}

/**
 * Merge Hub chat bubbles with local non-bubble cards (tool/git/system).
 *
 * Rules (final):
 * - Hub wins on **same message id** for chat content (multi-end SSOT).
 * - Hub `message_seq` is the **only** social total-order key for chat bubbles.
 * - Host tool/git/system/approval cards use `anchorHubMessageSeq` + `suborder`
 *   (never inject host daemon seq into social `messageSeq`).
 * - Local chat missing from Hub: optimistic / user / agent-result gap-fill only.
 */
export function mergeCloudAndLocalTimeline(input: {
  hubMessages: TimelineMessage[];
  localMessages: TimelineMessage[];
}): TimelineMessage[] {
  const byId = new Map<string, TimelineMessage>();
  const hubBubbles = input.hubMessages.filter(
    (m) => m.messageSeq != null && Number.isFinite(m.messageSeq),
  );

  // Local host-only cards with Hub-space anchors.
  for (const m of input.localMessages) {
    if (isLocalChatBubbleForCloudSsot(m)) continue;
    if (!isHostOnlyTimelineCard(m) && m.kind !== "system") {
      // Non-chat local rows that are not host cards (defensive).
      byId.set(m.id, m);
      continue;
    }
    const hostSeq =
      m.hostMessageSeq ??
      (m.messageSeq != null && Number.isFinite(m.messageSeq)
        ? m.messageSeq
        : undefined);
    const anchor =
      m.anchorHubMessageSeq ?? anchorForHostCard(m, hubBubbles);
    byId.set(m.id, {
      ...m,
      // Host cards never participate in Hub social messageSeq order.
      messageSeq: undefined,
      hostMessageSeq: hostSeq,
      anchorHubMessageSeq: anchor,
      suborder: m.suborder ?? hostSeq ?? 0,
    });
  }

  // Hub bubbles: content + Hub message_seq are authoritative.
  for (const m of input.hubMessages) {
    const localPeer = input.localMessages.find((l) => l.id === m.id);
    const reactions =
      m.reactions !== undefined ? m.reactions : localPeer?.reactions;
    const messageSeq =
      m.messageSeq != null && Number.isFinite(m.messageSeq)
        ? m.messageSeq
        : undefined;
    const createdAtMs =
      localPeer?.createdAtMs != null && Number.isFinite(localPeer.createdAtMs)
        ? localPeer.createdAtMs
        : m.createdAtMs;
    byId.set(m.id, {
      ...m,
      messageSeq,
      createdAtMs,
      reactions,
      // Chat bubbles are not host cards.
      anchorHubMessageSeq: undefined,
      hostMessageSeq: undefined,
      suborder: undefined,
    });
  }

  // Gap-fill local chat not yet on Hub.
  for (const m of input.localMessages) {
    if (!isLocalChatBubbleForCloudSsot(m)) continue;
    if (byId.has(m.id)) {
      const cur = byId.get(m.id)!;
      // Never overwrite Hub messageSeq with host daemon seq.
      if (cur.reactions === undefined && m.reactions !== undefined) {
        byId.set(m.id, { ...cur, reactions: m.reactions });
      }
      continue;
    }

    const keep =
      isOptimisticLocalBubble(m) ||
      m.role === "user" ||
      m.id.startsWith("agent-result:");
    if (keep) {
      // Pending uplink / optimistic: do not treat host daemon seq as Hub social
      // order when any Hub seq exists in the window.
      const hubSpaceActive = hubBubbles.length > 0;
      if (hubSpaceActive && !isOptimisticLocalBubble(m)) {
        byId.set(m.id, {
          ...m,
          hostMessageSeq: m.messageSeq,
          messageSeq: undefined,
        });
      } else {
        byId.set(m.id, m);
      }
    }
  }

  return sortTimelineMessages([...byId.values()]);
}

/** Apply one hub realtime message into an existing timeline window. */
export function upsertCloudMessageIntoTimeline(
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

  // Same-id supersede only (canonical agent-result ids — no session soft drop).
  byId.set(hub.id, hub);
  return sortTimelineMessages([...byId.values()]);
}
