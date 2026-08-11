/**
 * Sole write funnel for `messagesByConversation` (+ paired messageHistory).
 *
 * Callers (Hub bridge, loadTimeline, optimistic send) must not raw-set the
 * timeline map. Pure merges stay in shared/lib; this module owns the setState.
 */

import type { TimelineMessage } from "@/shared/domain/collaboration";
import {
  mergeCloudAndLocalTimeline,
  removeMessageFromTimeline,
  upsertCloudMessageIntoTimeline,
} from "@/shared/lib/cloud-timeline";
import {
  EMPTY_MESSAGE_HISTORY,
  mergeMessagesOlder,
  mergeMessagesQuietTail,
  messageHistoryFromWindow,
  trimMessagesHardMax,
  type MessageHistoryMeta,
} from "@/shared/lib/message-history";
import { useWorkspaceStore } from "@/store/workspace-store";

export type TimelineHistoryPatch = {
  hasOlderCloud?: boolean;
  hasOlderHost?: boolean;
  loadingOlder?: boolean;
};

/** Ensure a conversation window key exists (empty) without clobbering body. */
export function ensureTimelineWindowKey(conversationId: string): void {
  const id = conversationId.trim();
  if (!id) return;
  useWorkspaceStore.setState((s) => {
    if (
      Object.prototype.hasOwnProperty.call(s.messagesByConversation, id)
    ) {
      return {};
    }
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [id]: [],
      },
    };
  });
}

/** Hub live single-frame apply (same-id supersede + hard max trim). */
export function applyHubMessage(
  conversationId: string,
  message: TimelineMessage,
): void {
  const id = conversationId.trim();
  if (!id || !message.id) return;
  useWorkspaceStore.setState((s) => {
    const prev = s.messagesByConversation[id] ?? [];
    const merged = upsertCloudMessageIntoTimeline(prev, message);
    const trimmed = trimMessagesHardMax(merged);
    const prevHist =
      s.messageHistoryByConversation[id] ?? EMPTY_MESSAGE_HISTORY;
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [id]: trimmed.messages,
      },
      messageHistoryByConversation: {
        ...s.messageHistoryByConversation,
        [id]: messageHistoryFromWindow(trimmed.messages, {
          prev: prevHist,
          hasOlderCloud: prevHist.hasOlderCloud || trimmed.trimmed,
          hasOlderHost: prevHist.hasOlderHost,
          loadingOlder: false,
        }),
      },
    };
  });
}

/** Drop a recalled Hub message from the open window. */
export function removeRecalledMessage(
  conversationId: string,
  messageId: string,
): boolean {
  const id = conversationId.trim();
  const mid = messageId.trim();
  if (!id || !mid) return false;
  let changed = false;
  useWorkspaceStore.setState((s) => {
    if (
      !Object.prototype.hasOwnProperty.call(s.messagesByConversation, id)
    ) {
      return {};
    }
    const prev = s.messagesByConversation[id] ?? [];
    const next = removeMessageFromTimeline(prev, mid);
    if (next === prev) return {};
    changed = true;
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [id]: next,
      },
    };
  });
  return changed;
}

/**
 * Replace/merge a hydrate or snapshot window.
 * - `mode: "hub-local"` — Hub SSOT chat + local workbench cards
 * - `mode: "quiet-tail"` — union by id keeping older pages
 * - `mode: "older-prepend"` — page older into existing window
 */
export function replaceWindowFromHydrate(
  conversationId: string,
  input: {
    mode: "hub-local" | "quiet-tail" | "older-prepend";
    cloudMessages?: TimelineMessage[];
    localMessages?: TimelineMessage[];
    /** Existing window is read from store when omitted for older-prepend. */
    history?: TimelineHistoryPatch & {
      prev?: MessageHistoryMeta;
      /** Full history row override after merge (loadTimeline ready path). */
      nextHistory?: MessageHistoryMeta;
    };
  },
): void {
  const id = conversationId.trim();
  if (!id) return;
  useWorkspaceStore.setState((s) => {
    const prev = s.messagesByConversation[id] ?? [];
    const cloud = input.cloudMessages ?? [];
    const local = input.localMessages ?? [];
    let merged: TimelineMessage[];
    if (input.mode === "hub-local") {
      merged = mergeCloudAndLocalTimeline({
        cloudMessages: cloud,
        localMessages: local.length > 0 ? local : prev,
      });
    } else if (input.mode === "quiet-tail") {
      const withTail =
        prev.length > 0 ? mergeMessagesQuietTail(prev, cloud) : cloud;
      merged = mergeCloudAndLocalTimeline({
        cloudMessages: cloud,
        localMessages: withTail,
      });
    } else {
      // older-prepend
      let older = local;
      if (cloud.length > 0) {
        older = mergeCloudAndLocalTimeline({
          cloudMessages: cloud,
          localMessages: local,
        });
      }
      merged = mergeMessagesOlder(older, prev);
    }
    const trimmed = trimMessagesHardMax(merged);
    const prevHist =
      input.history?.prev ??
      s.messageHistoryByConversation[id] ??
      EMPTY_MESSAGE_HISTORY;
    const nextHistory =
      input.history?.nextHistory ??
      messageHistoryFromWindow(trimmed.messages, {
        prev: prevHist,
        hasOlderCloud:
          input.history?.hasOlderCloud ??
          (prevHist.hasOlderCloud || trimmed.trimmed),
        hasOlderHost:
          input.history?.hasOlderHost ??
          (prevHist.hasOlderHost || trimmed.trimmed),
        loadingOlder: input.history?.loadingOlder ?? false,
      });
    return {
      messagesByConversation: {
        ...s.messagesByConversation,
        [id]: trimmed.messages,
      },
      messageHistoryByConversation: {
        ...s.messageHistoryByConversation,
        [id]: nextHistory,
      },
    };
  });
}

/**
 * Install a fully merged window + history (loadTimeline ready path).
 * Caller runs pure merges; this is the only setState for the body.
 */
export function setTimelineWindow(
  conversationId: string,
  messages: TimelineMessage[],
  history: MessageHistoryMeta,
): void {
  const id = conversationId.trim();
  if (!id) return;
  useWorkspaceStore.setState((s) => ({
    messagesByConversation: {
      ...s.messagesByConversation,
      [id]: messages,
    },
    messageHistoryByConversation: {
      ...s.messageHistoryByConversation,
      [id]: history,
    },
  }));
}

/** Append optimistic user bubble (send path). */
export function applyOptimisticUserMessage(
  conversationId: string,
  message: TimelineMessage,
): void {
  const id = conversationId.trim();
  if (!id || !message.id) return;
  useWorkspaceStore.setState((s) => ({
    messagesByConversation: {
      ...s.messagesByConversation,
      [id]: [...(s.messagesByConversation[id] ?? []), message],
    },
    error: null,
  }));
}

/** Patch delivery / clock fields on one message by id. */
export function patchMessageDelivery(
  conversationId: string,
  messageId: string,
  patch: Partial<
    Pick<
      TimelineMessage,
      "deliveryStatus" | "messageSeq" | "time" | "createdAtMs"
    >
  >,
): void {
  const id = conversationId.trim();
  const mid = messageId.trim();
  if (!id || !mid) return;
  // Drop undefined keys so partial patches do not clobber existing fields.
  const clean: Partial<TimelineMessage> = {};
  if (patch.deliveryStatus !== undefined) {
    clean.deliveryStatus = patch.deliveryStatus;
  }
  if (patch.messageSeq !== undefined) clean.messageSeq = patch.messageSeq;
  if (patch.time !== undefined) clean.time = patch.time;
  if (patch.createdAtMs !== undefined) clean.createdAtMs = patch.createdAtMs;
  if (Object.keys(clean).length === 0) return;
  useWorkspaceStore.setState((s) => ({
    messagesByConversation: {
      ...s.messagesByConversation,
      [id]: (s.messagesByConversation[id] ?? []).map((m) =>
        m.id === mid ? { ...m, ...clean } : m,
      ),
    },
  }));
}
