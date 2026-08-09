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
  fanOutAgentTurns,
  resolveDispatchTargets,
} from "./send-dispatch";
import { syncUserMessageToCloud } from "@/shared/lib/im-cloud-sync";
import { isHubImMode } from "@/shared/lib/hub-timeline";
import { formatLocalClock } from "@/shared/lib/time";
import { hubDigestCache } from "@/shared/lib/hub-digest-cache";
import { positiveMs } from "@/shared/lib/rail-activity";
import { useAccountStore } from "@/store/account-store";

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
  hubDigestCache.patchOne(id, {
    preview: text,
    lastMessageAtMs: Math.max(
      clamped,
      positiveMs(hubDigestCache.get(id)?.lastMessageAtMs),
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
function hubAuthenticated(): boolean {
  const { session, authPhase } = useAccountStore.getState();
  return isHubImMode({
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

/** Load host profiles for @ProfileName / @p/id parse (best-effort). */
async function loadMentionProfiles(): Promise<MentionProfile[]> {
  try {
    const { profiles } = await daemonApi.listAgentProfiles();
    return (profiles ?? []).map((p) => ({
      id: p.id,
      name: p.name,
      runtimeAgent: p.runtime_agent,
    }));
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
    // P3 / C5.3: Intent Outbox — Hub HTTP + client_request_id when authenticated;
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
        route: hubAuthenticated() ? "hub" : "daemon",
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
    void import("@/shared/lib/hub-digest-cache").then(({ hubDigestCache }) => {
      hubDigestCache.patchOne(conversationId, {
        unreadCount: 0,
        unreadMentionCount: 0,
      });
    });
    // Phase 5.3: Linked / authenticated → Hub mark-read (multi-end inbox).
    // Only submit max *observed* Hub message_seq from the loaded timeline —
    // never server-latest (would silently mark unread rows as read).
    void import("@/shared/lib/minos-cloud").then(async (cloud) => {
      const { isHubImMode } = await import("@/shared/lib/hub-timeline");
      const { lastMessageSeq } = await import("@/shared/lib/message-history");
      const { useAccountStore } = await import("@/store/account-store");
      const { deviceId, session, authPhase } = useAccountStore.getState();
      if (
        !isHubImMode({
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
      const mentionProfiles = await loadMentionProfiles();
      const installedAgents = new Set(
        get()
          .clis.filter((c) => c.installed)
          .map((c) => c.agent.toLowerCase()),
      );
      const { targets, multiRoutedCount } = resolveDispatchTargets({
        messageBody,
        participatingAgents: conv.participatingAgents,
        installedAgents,
        mentionProfiles,
      });

      const convTitle = conv.title;
      const agentRuntimes = conv.participatingAgents;
      const accountOn = hubAuthenticated();

      // Desktop workbench always executes agents natively on this Host.
      // Linked/Hub is multi-end IM projection only — never the primary start path.
      // Append is idempotent by message_id. Success here only means durable.
      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        resolvedId,
      );
      patchDelivery("sent", messageSeq, clock);
      patchRailActivity(set, conversationId, railPreview, createdAtMs);
      if (accountOn) {
        void syncUserMessageToCloud({
          conversationId,
          messageId: resolvedId,
          text: messageBody,
          title: convTitle,
          replyToMessageId,
          createdAtMs,
          agentRuntimes,
          messageSource: "host_projection",
        });
      }
      void get().loadTimeline(conversationId, { quiet: true });

      const fanoutErrors = await fanOutAgentTurns({
        get,
        set,
        conversationId,
        workspacePath: project.workspacePath,
        messageBody,
        originMessageId: resolvedId,
        targets,
        multiRoutedCount,
      });
      if (fanoutErrors.length > 0) {
        set({
          actionError:
            fanoutErrors.length === targets.length
              ? fanoutErrors[0] ?? "Agent fan-out failed"
              : `Partial fan-out failure (${fanoutErrors.length}/${targets.length}): ${fanoutErrors[0]}`,
        });
      } else {
        set({ actionError: null });
      }

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
      if (fanoutErrors.length === targets.length) {
        throw new Error(fanoutErrors[0] ?? "Agent fan-out failed");
      }
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

      // Same pipeline as sendMessage: validate → append → sent → fan-out.
      const mentionProfiles = await loadMentionProfiles();
      const installedAgents = new Set(
        get()
          .clis.filter((c) => c.installed)
          .map((c) => c.agent.toLowerCase()),
      );
      const { targets, multiRoutedCount } = resolveDispatchTargets({
        messageBody,
        participatingAgents: conv.participatingAgents,
        installedAgents,
        mentionProfiles,
      });

      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        messageId,
      );
      const retryAt = failed.createdAtMs ?? Date.now();
      const retryClock = formatLocalClock(retryAt) || formatLocalClock(Date.now());
      patchDelivery("sent", messageSeq);
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
      if (hubAuthenticated()) {
        void syncUserMessageToCloud({
          conversationId,
          messageId,
          text: messageBody,
          title: conv.title,
          replyToMessageId,
          createdAtMs: retryAt,
          agentRuntimes: conv.participatingAgents,
          messageSource: "host_projection",
        });
      }
      void get().loadTimeline(conversationId, { quiet: true });

      const fanoutErrors = await fanOutAgentTurns({
        get,
        set,
        conversationId,
        workspacePath: project.workspacePath,
        messageBody,
        originMessageId: messageId,
        targets,
        multiRoutedCount,
      });
      if (fanoutErrors.length > 0) {
        set({
          actionError:
            fanoutErrors.length === targets.length
              ? fanoutErrors[0] ?? "Agent fan-out failed"
              : `Partial fan-out failure (${fanoutErrors.length}/${targets.length}): ${fanoutErrors[0]}`,
        });
      } else {
        set({ actionError: null });
      }

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
      if (fanoutErrors.length === targets.length) {
        throw new Error(fanoutErrors[0] ?? "Agent fan-out failed");
      }
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
