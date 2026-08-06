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
import {
  parseAgentRouting,
  parseAllAgentRoutings,
  type KnownAgent,
  type MentionProfile,
} from "@/shared/lib/agent-route";
import type { DeliveryStatus, TimelineMessage } from "@/shared/lib/mock-data";
import type { TranscriptItem } from "@/shared/lib/daemon";
import {
  ensureSessionsForRouting,
  quietRefreshConversationSlices,
  startNewAgentSession,
} from "./shared";
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
    await syncApprovalResolve({
      sessionId,
      requestId,
      decision: payload,
      clientOpId,
      route: hubAuthenticated() ? "hub" : "daemon",
    });
    // Drop local approval/question cards; Entity aggregates recompute via commit.
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
    // Pull fresh tail after the agent continues.
    await get().loadTranscript(sessionId, {
      tailWindow: 200,
      quiet: true,
      approvalStatusPolicy: "sync",
    });
  },

  respondOpencodePermission: async (sessionId, permissionId, response) => {
    if (get().source !== "daemon") return;
    await daemonApi.respondOpencodePermission(sessionId, permissionId, response);
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
    await get().loadTranscript(sessionId, {
      tailWindow: 200,
      quiet: true,
      approvalStatusPolicy: "sync",
    });
  },

  respondOpencodeQuestion: async (sessionId, questionId, answers) => {
    if (get().source !== "daemon") return;
    await daemonApi.respondOpencodeQuestion(sessionId, questionId, answers);
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
    await get().loadTranscript(sessionId, {
      tailWindow: 200,
      quiet: true,
      approvalStatusPolicy: "sync",
    });
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
    void import("@/shared/lib/minos-cloud").then(async (cloud) => {
      const { isHubImMode } = await import("@/shared/lib/hub-timeline");
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
      try {
        await cloud.markHubConversationRead(
          deviceId,
          session.accessToken,
          conversationId,
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

      const mentionProfiles = await loadMentionProfiles();
      const multiRouted = parseAllAgentRoutings(messageBody, mentionProfiles);
      const members = new Set(
        (conv.participatingAgents ?? []).map((a) => a.toLowerCase()),
      );

      // Multi-@ fan-out: one native session turn per unique mentioned agent.
      // Bare text (no @): single default member agent (legacy Desktop behavior).
      type DispatchTarget = {
        agent: KnownAgent;
        prompt: string;
        sessionShortId?: string;
        profileId?: string;
      };
      const targets: DispatchTarget[] = [];
      if (multiRouted.length > 0) {
        for (const routed of multiRouted) {
          const agent = routed.target.agent;
          if (!members.has(agent)) {
            throw new Error(
              members.size === 0
                ? "No agents in this conversation. Select agents when creating it before @mentioning."
                : `@${agent} is not a member of this conversation. Only roster agents can be @mentioned.`,
            );
          }
          targets.push({
            agent,
            prompt: routed.prompt,
            sessionShortId: routed.target.sessionShortId,
            profileId: routed.target.profileId,
          });
        }
      } else {
        const firstMember = (conv.participatingAgents ?? []).find((name) =>
          get().clis.some((c) => c.agent === name && c.installed),
        );
        const agent = (firstMember as KnownAgent | undefined) ?? null;
        if (!agent) {
          throw new Error(
            members.size === 0
              ? "No agents in this conversation. Select agents when creating it."
              : "No installed agents among conversation members. Install a member runtime or recreate with different agents.",
          );
        }
        if (!messageBody.trim()) {
          throw new Error("Cannot start an agent session with an empty prompt.");
        }
        targets.push({ agent, prompt: messageBody });
      }

      const convTitle = conv.title;
      const agentRuntimes = conv.participatingAgents;
      const accountOn = hubAuthenticated();

      // Desktop workbench always executes agents natively on this Host.
      // Linked/Hub is multi-end IM projection only — never the primary start path
      // (Hub try_agent_dispatch is for Mobile multi-@ fan-out).
      // Append is idempotent by message_id (store upsert). Constraint #3:
      // success here only means durable; later session steps may still throw.
      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        resolvedId,
      );
      patchDelivery("sent", messageSeq, clock);
      // Keep rail clock in sync even if Hub digest still lags.
      patchRailActivity(set, conversationId, railPreview, createdAtMs);
      // Multi-end visibility: project user bubble with host_projection so Hub
      // does not re-dispatch (this machine already owns execution).
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

      await ensureSessionsForRouting(get, conversationId);

      // Parallel fan-out after durable user bubble. Partial agent failures
      // must not flip the user row to failed (append already succeeded).
      const fanoutResults = await Promise.allSettled(
        targets.map(async (target) => {
          const sessions =
            get().sessionsByConversation[conversationId] ?? [];
          let sessionId: string | undefined;
          if (target.sessionShortId) {
            const match = sessions.find(
              (s) =>
                s.agent === target.agent &&
                s.status !== "done" &&
                (s.shortId === target.sessionShortId ||
                  s.id.endsWith(target.sessionShortId!) ||
                  s.id.startsWith(target.sessionShortId!)),
            );
            if (!match) {
              throw new Error(
                `No existing ${target.agent} session matches #${target.sessionShortId}`,
              );
            }
            sessionId = match.id;
          } else if (target.profileId) {
            sessionId = await startNewAgentSession(
              conversationId,
              target.agent,
              project.workspacePath,
              target.profileId,
            );
          } else {
            const reusable = sessions
              .filter(
                (s) =>
                  s.agent === target.agent &&
                  !s.parentId &&
                  s.status !== "done" &&
                  s.status !== "failed",
              )
              .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
            sessionId = reusable
              ? reusable.id
              : await startNewAgentSession(
                  conversationId,
                  target.agent,
                  project.workspacePath,
                );
          }

          // Full body for co-mention awareness (Hub multi-@ parity).
          const prompt =
            multiRouted.length > 0
              ? messageBody
              : target.prompt.trim() || messageBody;
          if (!prompt.trim()) {
            throw new Error(`Empty prompt for @${target.agent}`);
          }

          try {
            await daemonApi.resumeSession(sessionId, false);
            set((s) => {
              const prev = s.sessionsById[sessionId!];
              const row = findSessionRow(s, sessionId!);
              const entity = patchSessionEntity(prev, sessionId!, {
                daemonStatus: "running",
                needsContinue: false,
                conversationId:
                  prev?.conversationId ||
                  row?.conversationId ||
                  conversationId,
                agent: prev?.agent || row?.agent,
                shortId: prev?.shortId || row?.shortId,
                model: prev?.model || row?.model,
                summary: prev?.summary || row?.summary,
                parentId: prev?.parentId ?? row?.parentId,
                lastTsMs: Date.now(),
              });
              return commitSessionEntity(s, entity);
            });
          } catch {
            /* not needed when already live */
          }
          // Same origin message id for all agents in this multi-@ turn.
          await daemonApi.sendUserMessage(sessionId, prompt, resolvedId);
        }),
      );

      const fanoutErrors = fanoutResults
        .filter((r): r is PromiseRejectedResult => r.status === "rejected")
        .map((r) =>
          r.reason instanceof Error ? r.reason.message : String(r.reason),
        );
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
    // Constraint #2: never insert a new optimistic row on retry. The original
    // failed bubble already exists; reuse its message_id (store append is
    // idempotent by id) and patch delivery status in place.
    const list = get().messagesByConversation[conversationId] ?? [];
    const failed = list.find((m) => m.id === messageId);
    if (!failed) throw new Error("message not found");
    if (failed.deliveryStatus !== "failed") {
      throw new Error("message is not in a failed state");
    }
    const messageBody = failed.body;

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

    // Mock backend has no RPC: flip to sent synchronously.
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

      // Same as sendMessage: native local execution first; Hub is projection only.
      // Idempotent append by message_id — durable row (if any) is updated in
      // place rather than duplicated. This is the A9 main path.
      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        messageId,
      );
      const retryAt = failed.createdAtMs ?? Date.now();
      const retryClock = formatLocalClock(retryAt) || formatLocalClock(Date.now());
      patchDelivery("sent", messageSeq);
      // Ensure bubble shows a real local clock (legacy rows may still say "now").
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
          createdAtMs: retryAt,
          agentRuntimes: conv.participatingAgents,
          messageSource: "host_projection",
        });
      }
      void get().loadTimeline(conversationId, { quiet: true });

      const mentionProfiles = await loadMentionProfiles();
      const routed = parseAgentRouting(messageBody, mentionProfiles);
      let agent: KnownAgent | null = routed?.target.agent ?? null;
      const prompt = routed?.prompt ?? messageBody;
      const routeProfileId = routed?.target.profileId;
      const members = new Set(
        (conv.participatingAgents ?? []).map((a) => a.toLowerCase()),
      );
      if (agent && !members.has(agent)) {
        throw new Error(
          members.size === 0
            ? "No agents in this conversation. Select agents when creating it before @mentioning."
            : `@${agent} is not a member of this conversation. Only roster agents can be @mentioned.`,
        );
      }
      if (!agent) {
        const firstMember = (conv.participatingAgents ?? []).find((name) =>
          get().clis.some((c) => c.agent === name && c.installed),
        );
        agent = (firstMember as KnownAgent | undefined) ?? null;
      }
      if (!agent) {
        throw new Error(
          members.size === 0
            ? "No agents in this conversation. Select agents when creating it."
            : "No installed agents among conversation members. Install a member runtime or recreate with different agents.",
        );
      }

      // Re-run the session segment: resolve or create a thread, then deliver.
      let sessionId: string | undefined;
      await ensureSessionsForRouting(get, conversationId);
      const sessions = get().sessionsByConversation[conversationId] ?? [];
      if (routed?.target.sessionShortId) {
        const match = sessions.find(
          (s) =>
            s.agent === agent &&
            (s.shortId === routed.target.sessionShortId ||
              s.id.endsWith(routed.target.sessionShortId!) ||
              s.id.startsWith(routed.target.sessionShortId!)),
        );
        if (!match) {
          throw new Error(
            `No existing ${agent} session matches #${routed.target.sessionShortId}`,
          );
        }
        sessionId = match.id;
      } else if (routeProfileId) {
        sessionId = await startNewAgentSession(
          conversationId,
          agent,
          project.workspacePath,
          routeProfileId,
        );
      } else {
        const reusable = sessions
          .filter(
            (s) =>
              s.agent === agent &&
              !s.parentId &&
              s.status !== "done" &&
              s.status !== "failed",
          )
          .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
        if (reusable) {
          sessionId = reusable.id;
        } else {
          sessionId = await startNewAgentSession(
            conversationId,
            agent,
            project.workspacePath,
          );
        }
      }

      if (prompt.trim()) {
        try {
          await daemonApi.resumeSession(sessionId, false);
          set((s) => {
            const prev = s.sessionsById[sessionId];
            const row = findSessionRow(s, sessionId);
            const entity = patchSessionEntity(prev, sessionId, {
              daemonStatus: "running",
              needsContinue: false,
              conversationId:
                prev?.conversationId ||
                row?.conversationId ||
                conversationId,
              agent: prev?.agent || row?.agent,
              shortId: prev?.shortId || row?.shortId,
              model: prev?.model || row?.model,
              summary: prev?.summary || row?.summary,
              parentId: prev?.parentId ?? row?.parentId,
              lastTsMs: Date.now(),
            });
            return commitSessionEntity(s, entity);
          });
        } catch {
          /* not needed when already live */
        }
        await daemonApi.sendUserMessage(sessionId, prompt, messageId);
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
      set({ actionError: null });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      patchDelivery("failed");
      await quietRefreshConversationSlices(get, conversationId);
      throw e;
    }
  },
  };
}
