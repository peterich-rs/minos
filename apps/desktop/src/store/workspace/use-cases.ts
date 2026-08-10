/**
 * L6 use-cases — send, approvals, conversation/project mutations.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { commitSessionEntity, findSessionRow } from "./projection";
import { patchSessionEntity } from "@/shared/lib/session-entity";
import { daemonApi } from "@/shared/lib/daemon";
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";
import { transcriptHasPendingApproval } from "@/shared/lib/session-status";
import type { MentionProfile } from "@/shared/lib/agent-route";
import type { DeliveryStatus, TimelineMessage } from "@/shared/lib/mock-data";
import type { TranscriptItem } from "@/shared/lib/daemon";
import { quietRefreshConversationSlices } from "./shared";
import {
  buildStructuredMentions,
  resolveDispatchTargets,
} from "./send-dispatch";
import { syncUserMessageToCloud } from "@/shared/lib/im-cloud-sync";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import { formatLocalClock } from "@/shared/lib/time";
import { cloudDigestCache } from "@/shared/lib/cloud-digest-cache";
import { positiveMs } from "@/shared/lib/rail-activity";
import { useAccountStore } from "@/store/account-store";
import type { Conversation } from "@/shared/lib/mock-data";
import { membershipTokensOfBots } from "@/shared/lib/mock-data";

/**
 * Flattened membership tokens for resolveDispatchTargets.
 * Prefer `participatingBots` (roster SSOT); fall back to deprecated
 * `participatingAgents` only when bots array is empty (legacy daemon rows).
 */
function membershipTokensFromConv(conv: Conversation): string[] {
  const fromBots = membershipTokensOfBots(conv.participatingBots);
  if (fromBots.length > 0) return fromBots;
  return (conv.participatingAgents ?? [])
    .map((a) => a.trim().toLowerCase())
    .filter((a) => a.length > 0);
}

/** Preview + last-activity bump for the conversation rail (local-first). */
function patchRailActivity(
  set: WorkspaceSet,
  conversationId: string,
  preview: string,
  atMs: number,
): void {
  const id = conversationId.trim();
  if (!id) return;
  const text = preview.trim() || "No messages yet";
  const clamped = positiveMs(atMs);
  if (!clamped) return;
  cloudDigestCache.patchOne(id, {
    preview: text,
    lastMessageAtMs: Math.max(
      clamped,
      positiveMs(cloudDigestCache.get(id)?.lastMessageAtMs),
    ),
  });
  set((s) => ({
    conversations: s.conversations.map((c) =>
      c.id === id
        ? {
            ...c,
            preview: text,
            updatedAtMs: Math.max(positiveMs(c.updatedAtMs), clamped),
          }
        : c,
    ),
  }));
}
/** True when Minos account is signed in (multi-end Hub projection available). */
function cloudAuthenticated(): boolean {
  const { session, authPhase } = useAccountStore.getState();
  return isCloudImMode({
    authPhase,
    accessToken: session?.accessToken,
  });
}

/**
 * After local approval/permission resolution: patch Entity + recompute
 * conversation aggregates from Entity Σ (no ±1 approvalCount hacks).
 */
function commitResolvedApprovalState(
  s: WorkspaceState,
  sessionId: string,
  items: TranscriptItem[],
) {
  const stillPending = transcriptHasPendingApproval(items);
  const prevEntity = s.sessionsById[sessionId];
  const session = findSessionRow(s, sessionId);
  const entity = patchSessionEntity(prevEntity, sessionId, {
    hasPendingApproval: stillPending,
    daemonStatus: stillPending
      ? (prevEntity?.daemonStatus ?? "running")
      : "running",
    needsContinue: false,
    conversationId:
      session?.conversationId ?? prevEntity?.conversationId ?? "",
    agent: session?.agent ?? prevEntity?.agent,
    shortId: session?.shortId ?? prevEntity?.shortId,
    model: session?.model ?? prevEntity?.model,
    summary: session?.summary ?? prevEntity?.summary,
  });
  const committed = commitSessionEntity(s, entity);
  return {
    ...committed,
    transcriptsBySession: {
      ...s.transcriptsBySession,
      [sessionId]: items,
    },
  };
}

/**
 * Load mention profiles for dispatch parse.
 * Prefer Hub conversation participants (roster SSOT). Fall back to Host profiles
 * **filtered by conversation roster** — never the full unjoined profile directory.
 *
 * Membership tokens prefer `participatingBots` (botId ∪ name ∪ runtime);
 * `participatingAgents` is the deprecated runtime-label fallback only.
 */
async function loadMentionProfiles(
  conversationId: string,
  participatingAgents: string[] | undefined,
  participatingBots?: Array<{ botId: string; name: string; runtime: string }>,
): Promise<MentionProfile[]> {
  const memberSet = new Set<string>();
  for (const b of participatingBots ?? []) {
    for (const raw of [b.botId, b.name, b.runtime]) {
      const t = raw.trim().toLowerCase();
      if (t) memberSet.add(t);
    }
  }
  // Deprecated runtime labels only when bots did not contribute tokens.
  if (memberSet.size === 0) {
    for (const a of participatingAgents ?? []) {
      const t = a.trim().toLowerCase();
      if (t) memberSet.add(t);
    }
  }

  // Hub participants when account is online.
  try {
    const { useAccountStore } = await import("@/store/account-store");
    const { listConversationParticipants } = await import(
      "@/shared/lib/minos-cloud"
    );
    const { deviceId, session } = useAccountStore.getState();
    const token = session?.accessToken?.trim();
    if (token && conversationId) {
      const parts = await listConversationParticipants(
        deviceId,
        token,
        conversationId,
      );
      return parts.agents.map((a) => ({
        id: a.agentId,
        name: a.displayName || a.name || a.agentId,
        runtimeAgent: a.runtimeAgent,
      }));
    }
  } catch {
    /* fall through to host cache */
  }

  try {
    const { profiles } = await daemonApi.listAgentProfiles();
    const all = (profiles ?? []).map((p) => ({
      id: p.id,
      name: p.name,
      runtimeAgent: p.runtime_agent,
    }));
    // Roster-only: keep profiles whose runtime (or name/id) is a conversation member.
    if (memberSet.size === 0) return [];
    return all.filter(
      (p) =>
        memberSet.has(p.runtimeAgent.trim().toLowerCase()) ||
        memberSet.has(p.name.trim().toLowerCase()) ||
        memberSet.has(p.id.toLowerCase()),
    );
  } catch {
    return [];
  }
}


export function createUseCasesActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "resolveApproval"
  | "respondOpencodePermission"
  | "respondOpencodeQuestion"
  | "markConversationRead"
  | "sendMessage"
  | "retryFailedMessage"
> {
  return {
  resolveApproval: async (sessionId, requestId, decision) => {
    if (get().source !== "daemon") return;
    const payload =
      typeof decision === "string" ? { decision } : { ...decision };
    // Never inject client_request_id into agent decision (daemon/Hub strip).
    delete (payload as Record<string, unknown>).client_request_id;
    // Intent Outbox — Hub HTTP + client_request_id when authenticated;
    // otherwise local daemon path (decision JSON stays clean).
    const clientOpId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? `approval-${crypto.randomUUID()}`
        : `approval-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    const { syncApprovalResolve } = await import(
      "@/shared/lib/im-cloud-sync"
    );

    // Optimistic demote before RPC so the card leaves the actionable state
    // immediately (sendMessage delivery pattern). Restore on failure.
    const snapshotItems = get().transcriptsBySession[sessionId] ?? [];
    const snapshotEntity = get().sessionsById[sessionId];
    set((s) => {
      const items = (s.transcriptsBySession[sessionId] ?? []).map((it) =>
        it.requestId === requestId &&
        (it.kind === "approval" || it.kind === "question")
          ? {
              ...it,
              kind: "status" as const,
              text:
                typeof decision === "string"
                  ? `Answered: ${decision}`
                  : "Answered",
              title: "Resolved",
              requestId: null,
              options: null,
            }
          : it,
      );
      return commitResolvedApprovalState(s, sessionId, items);
    });

    try {
      await syncApprovalResolve({
        sessionId,
        requestId,
        decision: payload,
        clientOpId,
        route: cloudAuthenticated() ? "hub" : "daemon",
      });
      set({ actionError: null });
      // Pull fresh tail after the agent continues.
      await get().loadTranscript(sessionId, {
        tailWindow: 200,
        quiet: true,
        approvalStatusPolicy: "sync",
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) => {
        const committed =
          snapshotEntity != null
            ? commitSessionEntity(s, snapshotEntity)
            : {};
        return {
          ...committed,
          transcriptsBySession: {
            ...s.transcriptsBySession,
            [sessionId]: snapshotItems,
          },
          actionError: message,
        };
      });
      throw e;
    }
  },

  respondOpencodePermission: async (sessionId, permissionId, response) => {
    if (get().source !== "daemon") return;
    const snapshotItems = get().transcriptsBySession[sessionId] ?? [];
    const snapshotEntity = get().sessionsById[sessionId];
    set((s) => {
      const items = (s.transcriptsBySession[sessionId] ?? []).map((it) =>
        it.requestId === permissionId
          ? {
              ...it,
              kind: "status" as const,
              text: `Permission ${response}`,
              title: "Permission",
              requestId: null,
            }
          : it,
      );
      return commitResolvedApprovalState(s, sessionId, items);
    });
    try {
      await daemonApi.respondOpencodePermission(
        sessionId,
        permissionId,
        response,
      );
      set({ actionError: null });
      await get().loadTranscript(sessionId, {
        tailWindow: 200,
        quiet: true,
        approvalStatusPolicy: "sync",
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) => {
        const committed =
          snapshotEntity != null
            ? commitSessionEntity(s, snapshotEntity)
            : {};
        return {
          ...committed,
          transcriptsBySession: {
            ...s.transcriptsBySession,
            [sessionId]: snapshotItems,
          },
          actionError: message,
        };
      });
      throw e;
    }
  },

  respondOpencodeQuestion: async (sessionId, questionId, answers) => {
    if (get().source !== "daemon") return;
    const snapshotItems = get().transcriptsBySession[sessionId] ?? [];
    const snapshotEntity = get().sessionsById[sessionId];
    set((s) => {
      const items = (s.transcriptsBySession[sessionId] ?? []).map((it) =>
        it.requestId === questionId
          ? {
              ...it,
              kind: "status" as const,
              text: "Question answered",
              title: "Question",
              requestId: null,
              options: null,
            }
          : it,
      );
      return commitResolvedApprovalState(s, sessionId, items);
    });
    try {
      await daemonApi.respondOpencodeQuestion(sessionId, questionId, answers);
      set({ actionError: null });
      await get().loadTranscript(sessionId, {
        tailWindow: 200,
        quiet: true,
        approvalStatusPolicy: "sync",
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) => {
        const committed =
          snapshotEntity != null
            ? commitSessionEntity(s, snapshotEntity)
            : {};
        return {
          ...committed,
          transcriptsBySession: {
            ...s.transcriptsBySession,
            [sessionId]: snapshotItems,
          },
          actionError: message,
        };
      });
      throw e;
    }
  },

  markConversationRead: (conversationId) => {
    const conv = get().conversations.find((c) => c.id === conversationId);
    const count = conv?.messageCount ?? 0;
    // Focus + rail clear immediately (focused ≠ hasWindow).
    // Quiet loadTimeline must never set focus; only open / mark-read does.
    // P1: local baseline only matters for daemon-only; Hub path clears digest.
    set((s) => ({
      focusedConversationId: conversationId,
      readMessageCountById: {
        ...s.readMessageCountById,
        [conversationId]: count,
      },
      conversations: s.conversations.map((c) =>
        c.id === conversationId ? { ...c, unread: undefined } : c,
      ),
    }));
    void import("@/shared/lib/cloud-digest-cache").then(({ cloudDigestCache }) => {
      cloudDigestCache.patchOne(conversationId, {
        unreadCount: 0,
        unreadMentionCount: 0,
      });
    });
    // Linked / authenticated → Hub mark-read (multi-end inbox).
    // Only submit max *observed* Hub message_seq from the loaded timeline —
    // never server-latest (would silently mark unread rows as read).
    void import("@/shared/lib/minos-cloud").then(async (cloud) => {
      const { isCloudImMode } = await import("@/shared/lib/cloud-timeline");
      const { lastMessageSeq } = await import("@/shared/lib/message-history");
      const { useAccountStore } = await import("@/store/account-store");
      const { deviceId, session, authPhase } = useAccountStore.getState();
      if (
        !isCloudImMode({
          authPhase,
          accessToken: session?.accessToken,
        }) ||
        !session?.accessToken
      ) {
        return;
      }
      const timeline = get().messagesByConversation[conversationId] ?? [];
      const observedSeq = lastMessageSeq(timeline);
      if (observedSeq == null || observedSeq <= 0) {
        // No Hub seq loaded yet — skip HTTP; local rail already cleared.
        return;
      }
      try {
        await cloud.markHubConversationRead(
          deviceId,
          session.accessToken,
          conversationId,
          observedSeq,
        );
      } catch (error) {
        console.warn("[workspace] hub mark-read failed", error);
      }
    });
  },

  sendMessage: async (conversationId, body, messageId, options) => {
    const messageBody = body.trimEnd();
    if (!messageBody.trim()) return;

    // Constraint #1: generate messageId + optimistic `sending` insert BEFORE
    // any business validation that may throw, so the user always sees their
    // bubble immediately (WeChat: empty composer + sending row).
    const resolvedId = messageId ?? `msg_${crypto.randomUUID()}`;
    const replyToMessageId = options?.replyToMessageId;
    const createdAtMs = Date.now();
    const clock = formatLocalClock(createdAtMs);
    const optimistic: TimelineMessage = {
      id: resolvedId,
      role: "user",
      body: messageBody,
      time: clock,
      createdAtMs,
      deliveryStatus: "sending",
      // Wave 2: local reply attachment on optimistic UI. Daemon append does not
      // yet accept reply_to — durable round-trip deferred to protocol wave.
      ...(replyToMessageId ? { replyToMessageId } : {}),
    };
    // Optimistic timeline + rail (do not wait for Hub digest / list re-merge).
    set((s) => ({
      messagesByConversation: {
        ...s.messagesByConversation,
        [conversationId]: [
          ...(s.messagesByConversation[conversationId] ?? []),
          optimistic,
        ],
      },
      error: null,
    }));
    const railPreview =
      messageBody.trim().length > 88
        ? `${messageBody.trim().slice(0, 88)}…`
        : messageBody.trim();
    patchRailActivity(set, conversationId, railPreview, createdAtMs);

    const patchDelivery = (
      status: DeliveryStatus,
      seq?: number,
      time?: string,
    ) => {
      set((s) => ({
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: (s.messagesByConversation[conversationId] ?? []).map(
            (m) =>
              m.id === resolvedId
                ? {
                    ...m,
                    deliveryStatus: status,
                    messageSeq: seq ?? m.messageSeq,
                    time: time ?? m.time,
                    createdAtMs: m.createdAtMs ?? createdAtMs,
                  }
                : m,
          ),
        },
      }));
    };

    // Mock backend has no RPC: flip to sent synchronously.
    if (get().source !== "daemon") {
      patchDelivery("sent", undefined, clock);
      return;
    }

    try {
      const conv = get().conversations.find((c) => c.id === conversationId);
      const project = get().projects.find((p) => p.id === conv?.projectId);
      if (!conv || !project) {
        throw new Error("conversation or project not found");
      }

      // Validate routing BEFORE durable append (same order as retry).
      // mentionProfiles are roster-scoped (Hub participants preferred).
      const mentionProfiles = await loadMentionProfiles(
        conversationId,
        conv.participatingAgents,
        conv.participatingBots,
      );
      const installedAgents = new Set(
        get()
          .clis.filter((c) => c.installed)
          .map((c) => c.agent.toLowerCase()),
      );
      const membershipTokens = membershipTokensFromConv(conv);
      // Validate @ targets (membership / sole-route). Bot activation itself is
      // Hub-only when Account is live — no local startAgent fan-out.
      resolveDispatchTargets({
        messageBody,
        participatingAgents: membershipTokens,
        installedAgents,
        mentionProfiles,
      });
      // Structured mentions for AppendMessage (Hub validates membership only).
      const structuredMentions = buildStructuredMentions(
        messageBody,
        mentionProfiles,
      );

      const convTitle = conv.title;
      // Host runtime bins for cloud upsert — derived from bot roster SSOT.
      const agentRuntimes = (() => {
        const fromBots = conv.participatingBots
          ?.map((b) => b.runtime.trim().toLowerCase())
          .filter(Boolean);
        if (fromBots && fromBots.length > 0) return fromBots;
        return conv.participatingAgents;
      })();
      const accountOn = cloudAuthenticated();
      if (!accountOn) {
        throw new Error(
          "Sign in to send collaboration messages. Bot delivery requires Account Hub sync.",
        );
      }

      // Collaboration write authority is Hub only. "sent" comes from ChatSendAck
      // (or durable echo); local daemon workbench is a projection after Ack.
      const hubAck = await syncUserMessageToCloud({
        conversationId,
        messageId: resolvedId,
        text: messageBody,
        title: convTitle,
        replyToMessageId,
        createdAtMs,
        agentRuntimes,
        messageSource: "client_live",
        mentions:
          structuredMentions.length > 0 ? structuredMentions : undefined,
      });
      const messageSeq = hubAck?.messageSeq;
      patchDelivery("sent", messageSeq, clock);
      patchRailActivity(set, conversationId, railPreview, createdAtMs);
      // Optional local workbench projection (idempotent by message_id) — not write authority.
      try {
        await daemonApi.appendUserMessage(
          conversationId,
          messageBody,
          resolvedId,
        );
      } catch (localErr) {
        console.warn(
          "[workspace] local workbench projection after Hub Ack failed",
          localErr,
        );
      }
      set({ actionError: null });
      void get().loadTimeline(conversationId, { quiet: true });

      await quietRefreshConversationSlices(get, conversationId);
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.conversations(conv.projectId),
      });
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.projectSessions(conv.projectId),
      });
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.inspectorSessions(conversationId),
      });
      await get().loadConversations(conv.projectId);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      // Only mark user bubble failed when append itself failed (still "sending").
      const stillSending = (get().messagesByConversation[conversationId] ?? [])
        .find((m) => m.id === resolvedId)
        ?.deliveryStatus === "sending";
      if (stillSending) {
        patchDelivery("failed");
      }
      await quietRefreshConversationSlices(get, conversationId);
      throw e;
    }
  },

  retryFailedMessage: async (conversationId, messageId) => {
    // Constraint #2: never insert a new optimistic row on retry. Reuse
    // message_id (store append is idempotent) and patch delivery in place.
    const list = get().messagesByConversation[conversationId] ?? [];
    const failed = list.find((m) => m.id === messageId);
    if (!failed) throw new Error("message not found");
    if (failed.deliveryStatus !== "failed") {
      throw new Error("message is not in a failed state");
    }
    const messageBody = failed.body;
    const replyToMessageId = failed.replyToMessageId;

    const patchDelivery = (
      status: DeliveryStatus,
      seq?: number,
    ) => {
      set((s) => ({
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: (s.messagesByConversation[conversationId] ?? []).map(
            (m) =>
              m.id === messageId
                ? {
                    ...m,
                    deliveryStatus: status,
                    messageSeq: seq ?? m.messageSeq,
                  }
                : m,
          ),
        },
      }));
    };

    patchDelivery("sending");

    if (get().source !== "daemon") {
      patchDelivery("sent");
      return;
    }

    try {
      const conv = get().conversations.find((c) => c.id === conversationId);
      const project = get().projects.find((p) => p.id === conv?.projectId);
      if (!conv || !project) {
        throw new Error("conversation or project not found");
      }

      // Same pipeline as sendMessage: validate → append → Hub uplink.
      const mentionProfiles = await loadMentionProfiles(
        conversationId,
        conv.participatingAgents,
        conv.participatingBots,
      );
      const installedAgents = new Set(
        get()
          .clis.filter((c) => c.installed)
          .map((c) => c.agent.toLowerCase()),
      );
      resolveDispatchTargets({
        messageBody,
        participatingAgents: membershipTokensFromConv(conv),
        installedAgents,
        mentionProfiles,
      });
      const structuredMentions = buildStructuredMentions(
        messageBody,
        mentionProfiles,
      );
      if (!cloudAuthenticated()) {
        throw new Error(
          "Sign in to send collaboration messages. Bot delivery requires Account Hub sync.",
        );
      }

      const retryAt = failed.createdAtMs ?? Date.now();
      const retryClock = formatLocalClock(retryAt) || formatLocalClock(Date.now());
      const hubAck = await syncUserMessageToCloud({
        conversationId,
        messageId,
        text: messageBody,
        title: conv.title,
        replyToMessageId,
        createdAtMs: retryAt,
        agentRuntimes: (() => {
          const fromBots = conv.participatingBots
            ?.map((b) => b.runtime.trim().toLowerCase())
            .filter(Boolean);
          if (fromBots && fromBots.length > 0) return fromBots;
          return conv.participatingAgents;
        })(),
        messageSource: "client_live",
        mentions:
          structuredMentions.length > 0 ? structuredMentions : undefined,
      });
      patchDelivery("sent", hubAck?.messageSeq);
      set((s) => ({
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: (s.messagesByConversation[conversationId] ?? []).map(
            (m) =>
              m.id === messageId
                ? { ...m, time: retryClock, createdAtMs: retryAt }
                : m,
          ),
        },
      }));
      const railPreview =
        messageBody.trim().length > 88
          ? `${messageBody.trim().slice(0, 88)}…`
          : messageBody.trim();
      patchRailActivity(set, conversationId, railPreview, retryAt);
      try {
        await daemonApi.appendUserMessage(
          conversationId,
          messageBody,
          messageId,
        );
      } catch (localErr) {
        console.warn(
          "[workspace] local workbench projection after Hub Ack failed",
          localErr,
        );
      }
      set({ actionError: null });
      void get().loadTimeline(conversationId, { quiet: true });

      await quietRefreshConversationSlices(get, conversationId);
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.conversations(conv.projectId),
      });
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.projectSessions(conv.projectId),
      });
      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.inspectorSessions(conversationId),
      });
      await get().loadConversations(conv.projectId);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      // Only flip to failed when still sending (append never succeeded).
      const stillSending = (get().messagesByConversation[conversationId] ?? [])
        .find((m) => m.id === messageId)
        ?.deliveryStatus === "sending";
      if (stillSending) {
        patchDelivery("failed");
      }
      await quietRefreshConversationSlices(get, conversationId);
      throw e;
    }
  },
  };
}
