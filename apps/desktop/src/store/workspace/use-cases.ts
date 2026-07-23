/**
 * L6 use-cases — send, approvals, conversation/project mutations.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  patchLocalConversation,
  toUiConversation,
  toUiProject,
} from "./helpers";
import {
  commitSessionEntity,
  findSessionRow,
  quietHydrateAllConversationLists,
} from "./projection";
import { patchSessionEntity } from "@/shared/lib/session-entity";
import { daemonApi } from "@/shared/lib/daemon";
import { transcriptHasPendingApproval } from "@/shared/lib/session-status";
import {
  parseAgentRouting,
  type KnownAgent,
  type MentionProfile,
} from "@/shared/lib/agent-route";
import {
  nextPriority,
  nextProgress,
  parsePriority,
  parseProgress,
  progressForBoardColumn,
} from "@/shared/lib/conversation-meta";
import type {
  Conversation,
  DeliveryStatus,
  Project,
  TimelineMessage,
} from "@/shared/lib/mock-data";
import {
  ensureSessionsForRouting,
  quietRefreshConversationSlices,
  startNewAgentSession,
} from "./shared";

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
  | "createConversation"
  | "updateConversationTitle"
  | "cycleConversationPriority"
  | "cycleConversationProgress"
  | "setConversationProgress"
  | "moveConversationToBoardColumn"
  | "createProject"
> {
  return {
  resolveApproval: async (sessionId, requestId, decision) => {
    if (get().source !== "daemon") return;
    const payload =
      typeof decision === "string" ? { decision } : decision;
    await daemonApi.resolveApproval(requestId, sessionId, payload);
    // Drop local approval/question cards for this request; re-list will converge.
    set((s) => {
      const items = (s.transcriptsBySession[sessionId] ?? []).map((it) =>
        it.requestId === requestId &&
        (it.kind === "approval" || it.kind === "question")
          ? {
              ...it,
              kind: "status",
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
      const stillPending = transcriptHasPendingApproval(items);
      const prevEntity = s.sessionsById[sessionId];
      const session = findSessionRow(s, sessionId);
      const entity = patchSessionEntity(prevEntity, sessionId, {
        hasPendingApproval: stillPending,
        daemonStatus: stillPending
          ? (prevEntity?.daemonStatus ?? "running")
          : "running",
        conversationId:
          session?.conversationId ?? prevEntity?.conversationId ?? "",
        agent: session?.agent ?? prevEntity?.agent,
        shortId: session?.shortId ?? prevEntity?.shortId,
        model: session?.model ?? prevEntity?.model,
        summary: session?.summary ?? prevEntity?.summary,
      });
      const committed = commitSessionEntity(s, entity, {
        elevateApprovalCount: false,
      });
      const convId = entity.conversationId;
      const wasPending = prevEntity?.hasPendingApproval === true;
      let conversations = committed.conversations;
      // Optimistic: clear one approval slot when last pending is resolved.
      if (convId && wasPending && !stillPending) {
        conversations = patchLocalConversation(conversations, convId, {
          approvalCount: Math.max(
            (s.conversations.find((c) => c.id === convId)?.approvalCount ?? 1) -
              1,
            0,
          ),
        });
      }
      return {
        ...committed,
        conversations,
        transcriptsBySession: {
          ...s.transcriptsBySession,
          [sessionId]: items,
        },
      };
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
    // Same Entity + list projection path as resolveApproval (status sole writers).
    set((s) => {
      const items = (s.transcriptsBySession[sessionId] ?? []).map((it) =>
        it.requestId === permissionId
          ? {
              ...it,
              kind: "status",
              text: `Permission ${response}`,
              title: "Permission",
              requestId: null,
            }
          : it,
      );
      const stillPending = transcriptHasPendingApproval(items);
      const prevEntity = s.sessionsById[sessionId];
      const session = findSessionRow(s, sessionId);
      const entity = patchSessionEntity(prevEntity, sessionId, {
        hasPendingApproval: stillPending,
        daemonStatus: stillPending
          ? (prevEntity?.daemonStatus ?? "running")
          : "running",
        conversationId:
          session?.conversationId ?? prevEntity?.conversationId ?? "",
        agent: session?.agent ?? prevEntity?.agent,
        shortId: session?.shortId ?? prevEntity?.shortId,
        model: session?.model ?? prevEntity?.model,
        summary: session?.summary ?? prevEntity?.summary,
      });
      const committed = commitSessionEntity(s, entity, {
        elevateApprovalCount: false,
      });
      const convId = entity.conversationId;
      const wasPending = prevEntity?.hasPendingApproval === true;
      let conversations = committed.conversations;
      if (convId && wasPending && !stillPending) {
        conversations = patchLocalConversation(conversations, convId, {
          approvalCount: Math.max(
            (s.conversations.find((c) => c.id === convId)?.approvalCount ?? 1) -
              1,
            0,
          ),
        });
      }
      return {
        ...committed,
        conversations,
        transcriptsBySession: {
          ...s.transcriptsBySession,
          [sessionId]: items,
        },
      };
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
              kind: "status",
              text: "Question answered",
              title: "Question",
              requestId: null,
              options: null,
            }
          : it,
      );
      const stillPending = transcriptHasPendingApproval(items);
      const prevEntity = s.sessionsById[sessionId];
      const session = findSessionRow(s, sessionId);
      const entity = patchSessionEntity(prevEntity, sessionId, {
        hasPendingApproval: stillPending,
        daemonStatus: stillPending
          ? (prevEntity?.daemonStatus ?? "running")
          : "running",
        conversationId:
          session?.conversationId ?? prevEntity?.conversationId ?? "",
        agent: session?.agent ?? prevEntity?.agent,
        shortId: session?.shortId ?? prevEntity?.shortId,
        model: session?.model ?? prevEntity?.model,
        summary: session?.summary ?? prevEntity?.summary,
      });
      const committed = commitSessionEntity(s, entity, {
        elevateApprovalCount: false,
      });
      const convId = entity.conversationId;
      const wasPending = prevEntity?.hasPendingApproval === true;
      let conversations = committed.conversations;
      if (convId && wasPending && !stillPending) {
        conversations = patchLocalConversation(conversations, convId, {
          approvalCount: Math.max(
            (s.conversations.find((c) => c.id === convId)?.approvalCount ?? 1) -
              1,
            0,
          ),
        });
      }
      return {
        ...committed,
        conversations,
        transcriptsBySession: {
          ...s.transcriptsBySession,
          [sessionId]: items,
        },
      };
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
  },

  sendMessage: async (conversationId, body, messageId, options) => {
    const messageBody = body.trimEnd();
    if (!messageBody.trim()) return;

    // Constraint #1: generate messageId + optimistic `sending` insert BEFORE
    // any business validation that may throw, so the user always sees their
    // bubble immediately (WeChat: empty composer + sending row).
    const resolvedId = messageId ?? `msg_${crypto.randomUUID()}`;
    const replyToMessageId = options?.replyToMessageId;
    const optimistic: TimelineMessage = {
      id: resolvedId,
      role: "user",
      body: messageBody,
      time: "now",
      createdAtMs: Date.now(),
      deliveryStatus: "sending",
      // Wave 2: local reply attachment on optimistic UI. Daemon append does not
      // yet accept reply_to — durable round-trip deferred to protocol wave.
      ...(replyToMessageId ? { replyToMessageId } : {}),
    };
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
                  }
                : m,
          ),
        },
      }));
    };

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

      const mentionProfiles = await loadMentionProfiles();
      const routed = parseAgentRouting(messageBody, mentionProfiles);
      let agent: KnownAgent | null = routed?.target.agent ?? null;
      let prompt = routed?.prompt ?? messageBody;
      const routeProfileId = routed?.target.profileId;

      if (!agent) {
        const firstOk = get().clis.find((c) => c.installed);
        agent = (firstOk?.agent as KnownAgent | undefined) ?? null;
        prompt = messageBody;
      }
      if (!agent) {
        throw new Error(
          "No agents available. Install codex/claude/gemini/opencode/grok.",
        );
      }
      if (!prompt.trim() && !routed) {
        throw new Error("Cannot start an agent session with an empty prompt.");
      }

      // Append is idempotent by message_id (store upsert). Constraint #3:
      // success here only means durable; later session steps may still throw,
      // in which case the row stays `failed` but the message may be durable.
      // That is the defined send-pipeline-failed semantics.
      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        resolvedId,
      );
      patchDelivery("sent", messageSeq);

      let sessionId: string | undefined;
      // Use-case: need session list for reuse / #short before routing.
      await ensureSessionsForRouting(get, conversationId);
      const sessions = get().sessionsByConversation[conversationId] ?? [];
      if (routed?.target.sessionShortId) {
        const match = sessions.find(
          (s) =>
            s.agent === agent &&
            s.status !== "done" &&
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
        // Explicit profile mention always starts a new session (create-time bind).
        sessionId = await startNewAgentSession(
          conversationId,
          agent,
          project.workspacePath,
          routeProfileId,
        );
      } else {
        // Reuse most recent non-closed session for this agent when present
        // (parity with continuing the same session after TUI/Desktop restart).
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
        // Reattach only — user text wins over any pending auto-continue flag.
        try {
          await daemonApi.resumeSession(sessionId, false);
        } catch {
          /* not needed when already live */
        }
        await daemonApi.sendUserMessage(sessionId, prompt);
      }

      await quietRefreshConversationSlices(get, conversationId);
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

      // Idempotent append by message_id — durable row (if any) is updated in
      // place rather than duplicated. This is the A9 main path.
      const { messageSeq } = await daemonApi.appendUserMessage(
        conversationId,
        messageBody,
        messageId,
      );
      patchDelivery("sent", messageSeq);

      const mentionProfiles = await loadMentionProfiles();
      const routed = parseAgentRouting(messageBody, mentionProfiles);
      let agent: KnownAgent | null = routed?.target.agent ?? null;
      const prompt = routed?.prompt ?? messageBody;
      const routeProfileId = routed?.target.profileId;
      if (!agent) {
        const firstOk = get().clis.find((c) => c.installed);
        agent = (firstOk?.agent as KnownAgent | undefined) ?? null;
      }
      if (!agent) {
        throw new Error(
          "No agents available. Install codex/claude/gemini/opencode/grok.",
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
        } catch {
          /* not needed when already live */
        }
        await daemonApi.sendUserMessage(sessionId, prompt);
      }

      await quietRefreshConversationSlices(get, conversationId);
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

  createConversation: async (projectId, title) => {
    if (get().source !== "daemon") {
      // Mock: append a local conversation for browser-only preview.
      const id = `mock-conv-${Date.now()}`;
      const conv: Conversation = {
        id,
        projectId,
        title,
        preview: "No messages yet",
        updatedAt: "now",
        updatedAtMs: Date.now(),
        messageCount: 0,
        boardColumn: "backlog",
        agentSessionCount: 0,
        runningCount: 0,
        approvalCount: 0,
        progress: "todo",
      };
      set((s) => ({ conversations: [conv, ...s.conversations] }));
      return id;
    }
    const created = await daemonApi.createConversation(projectId, title);
    await get().loadConversations(projectId);
    return created.id;
  },

  updateConversationTitle: async (conversationId, title) => {
    const trimmed = title.trim();
    if (!trimmed) {
      throw new Error("title cannot be empty");
    }
    if (get().source !== "daemon") {
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          title: trimmed,
        }),
      }));
      return;
    }
    const updated = await daemonApi.updateConversation(conversationId, {
      title: trimmed,
    });
    set((s) => ({
      conversations: patchLocalConversation(s.conversations, conversationId, {
        ...toUiConversation(
          updated,
          s.readMessageCountById,
          s.focusedConversationId,
        ),
        // Preserve local attention fields that RPC may not recompute yet.
        unread: s.conversations.find((c) => c.id === conversationId)?.unread,
      }),
    }));
  },

  cycleConversationPriority: async (conversationId) => {
    const current = get().conversations.find((c) => c.id === conversationId);
    if (!current) return;
    const next = nextPriority(current.priority);
    const priorityValue = next ?? "";
    if (get().source !== "daemon") {
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          priority: next ?? undefined,
        }),
      }));
      return;
    }
    const updated = await daemonApi.updateConversation(conversationId, {
      priority: priorityValue,
    });
    set((s) => ({
      conversations: patchLocalConversation(s.conversations, conversationId, {
        priority: parsePriority(updated.priority),
      }),
    }));
  },

  cycleConversationProgress: async (conversationId) => {
    const current = get().conversations.find((c) => c.id === conversationId);
    if (!current) return;
    const next = nextProgress(current.progress);
    if (get().source !== "daemon") {
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          progress: next,
        }),
      }));
      return;
    }
    const updated = await daemonApi.updateConversation(conversationId, {
      progress: next,
    });
    set((s) => ({
      conversations: patchLocalConversation(s.conversations, conversationId, {
        progress: parseProgress(updated.progress),
      }),
    }));
  },

  setConversationProgress: async (conversationId, progress) => {
    if (get().source !== "daemon") {
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          progress,
        }),
      }));
      return;
    }
    const updated = await daemonApi.updateConversation(conversationId, {
      progress,
    });
    set((s) => ({
      conversations: patchLocalConversation(s.conversations, conversationId, {
        progress: parseProgress(updated.progress),
      }),
    }));
  },

  moveConversationToBoardColumn: async (conversationId, column) => {
    const progress = progressForBoardColumn(column);
    await get().setConversationProgress(conversationId, progress);
  },

  createProject: async (workspacePath) => {
    const trimmed = workspacePath.trim();
    if (!trimmed) {
      throw new Error("workspace path is required");
    }

    if (get().source !== "daemon" || !get().connection?.connected) {
      const base =
        trimmed.split(/[/\\]/).filter(Boolean).pop() || "project";
      const project: Project = {
        id: `mock-proj-${Date.now()}`,
        name: base,
        workspacePath: trimmed,
        conversationCount: 0,
        runningAgents: 0,
        needsAttention: 0,
        updatedAtMs: Date.now(),
        hasUnread: false,
        lastAttentionMs: 0,
      };
      set((s) => ({
        projects: [...s.projects, project],
        error: null,
        source: "mock",
      }));
      return project.id;
    }

    try {
      const created = toUiProject(await daemonApi.createProject(trimmed));
      const projects = (await daemonApi.listProjects()).map(toUiProject);
      set({ projects, actionError: null });
      void quietHydrateAllConversationLists(get);
      return created.id;
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      throw e;
    }
  },
  };
}
