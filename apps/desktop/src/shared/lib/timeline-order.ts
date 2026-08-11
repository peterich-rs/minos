import type { TimelineMessage } from "../domain/collaboration.ts";

/** Host workbench cards that hang off Hub social order via anchor+suborder. */
export function isHostOnlyTimelineCard(message: TimelineMessage): boolean {
  const kind = message.kind;
  return (
    kind === "tool_summary" ||
    kind === "git_activity" ||
    kind === "approval" ||
    kind === "system"
  );
}

function isOptimisticNoSeq(message: TimelineMessage): boolean {
  if (isHostOnlyTimelineCard(message)) return false;
  return (
    (message.deliveryStatus === "sending" ||
      message.deliveryStatus === "failed") &&
    !(message.messageSeq != null && Number.isFinite(message.messageSeq))
  );
}

type SortKey = {
  /** 0 = durable social/host; 1 = optimistic tail */
  domain: number;
  /** Hub message_seq or anchorCloudMessageSeq */
  primary: number;
  /** 0 for chat bubbles; host card suborder after same primary */
  secondary: number;
  createdAtMs: number;
  id: string;
};

function sortKey(message: TimelineMessage): SortKey {
  if (isOptimisticNoSeq(message)) {
    return {
      domain: 1,
      primary: message.createdAtMs ?? 0,
      secondary: 0,
      createdAtMs: message.createdAtMs ?? 0,
      id: message.id,
    };
  }

  if (isHostOnlyTimelineCard(message)) {
    const anchor = message.anchorCloudMessageSeq;
    if (anchor != null && Number.isFinite(anchor)) {
      return {
        domain: 0,
        primary: anchor,
        // Host cards after the bubble with the same hub seq.
        secondary: (message.suborder ?? 0) + 1,
        createdAtMs: message.createdAtMs ?? 0,
        id: message.id,
      };
    }
    // Unanchored host card: after all hub-seq social rows, by hostMessageSeq.
    return {
      domain: 0,
      primary: Number.MAX_SAFE_INTEGER / 2,
      secondary: message.hostMessageSeq ?? message.suborder ?? 0,
      createdAtMs: message.createdAtMs ?? 0,
      id: message.id,
    };
  }

  // Chat bubble: Hub (or local-only host) messageSeq is the social key.
  if (message.messageSeq != null && Number.isFinite(message.messageSeq)) {
    return {
      domain: 0,
      primary: message.messageSeq,
      secondary: 0,
      createdAtMs: message.createdAtMs ?? 0,
      id: message.id,
    };
  }

  // Durable chat without seq (gap-fill pending uplink): before optimistic tail,
  // after numbered social rows, by createdAt then id.
  return {
    domain: 0,
    primary: Number.MAX_SAFE_INTEGER / 4,
    secondary: 0,
    createdAtMs: message.createdAtMs ?? 0,
    id: message.id,
  };
}

function compareKeys(a: SortKey, b: SortKey): number {
  if (a.domain !== b.domain) return a.domain - b.domain;
  if (a.primary !== b.primary) return a.primary - b.primary;
  if (a.secondary !== b.secondary) return a.secondary - b.secondary;
  if (a.createdAtMs !== b.createdAtMs) return a.createdAtMs - b.createdAtMs;
  return a.id.localeCompare(b.id);
}

/**
 * Canonical conversation timeline order.
 *
 * Linked (Hub SSOT):
 * - Chat bubbles order by Hub `messageSeq` ASC.
 * - Host-only cards order by `anchorCloudMessageSeq` then `suborder` (after anchor bubble).
 * - Optimistic sending/failed without seq form a separate tail domain.
 *
 * Never uses wall-clock as the primary durable social order domain.
 */
export function sortTimelineMessages(
  messages: TimelineMessage[],
): TimelineMessage[] {
  return [...messages].sort((a, b) => compareKeys(sortKey(a), sortKey(b)));
}
