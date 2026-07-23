/**
 * Shared pure helpers for workspace slices (DTO mapping, empty caches, mock).
 */
import {
  conversations as mockConversations,
  projects as mockProjects,
  timelineByConversation as mockTimeline,
  agentSessions as mockSessions,
  type Conversation,
  type Project,
  type TimelineMessage,
  type AgentRuntime,
  type SessionStatus,
} from "@/shared/lib/mock-data";
import {
  type DaemonConversation,
  type DaemonMessage,
  type DaemonProject,
  type DaemonSession,
  type TranscriptItem,
} from "@/shared/lib/daemon";
import {
  deriveBoardColumn,
  parsePriority,
  parseProgress,
} from "@/shared/lib/conversation-meta";
import {
  entityNeedsAttention,
  mergeSessionEntity,
  projectSessionFromEntity,
  type SessionEntity,
} from "@/shared/lib/session-entity";
import { formatLocalClock, formatRelative } from "@/shared/lib/time";
import type { MessageHistoryMeta } from "@/shared/lib/message-history";
import type { TranscriptHistoryMeta } from "@/shared/lib/transcript-history";
import { transcriptItemEqual } from "@/shared/lib/list-identity";
import type {
  ProjectSession,
  ResourceFetchPhase,
  ResourceFetchStatus,
  WorkspaceState,
} from "./types";

/** Debounced quiet Timeline re-list timers (conversationId → timeout handle). */
export const conversationRefreshTimers = new Map<
  string,
  ReturnType<typeof setTimeout>
>();

/**
 * Mock / offline CLI inventory shaped like daemon `list_clis`.
 * Capability flags mirror domain SSOT (Codex/Grok effort; all model selection).
 * Not a second capability table for production — daemon inventory replaces this.
 */
export const KNOWN_AGENTS_FALLBACK: {
  agent: string;
  displayName: string;
  installed: boolean;
  status: string;
  supportsModelSelection: boolean;
  supportsReasoningEffort: boolean;
}[] = [
  {
    agent: "codex",
    displayName: "Codex",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
  {
    agent: "claude",
    displayName: "Claude",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "gemini",
    displayName: "Gemini",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "opencode",
    displayName: "OpenCode",
    installed: false,
    status: "missing",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "grok",
    displayName: "Grok",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
];

/** In-flight bootstrap so React StrictMode double-mount cannot wipe loads. */
let bootstrapInFlight: Promise<void> | null = null;

export function getBootstrapInFlight(): Promise<void> | null {
  return bootstrapInFlight;
}

export function setBootstrapInFlight(p: Promise<void> | null): void {
  bootstrapInFlight = p;
}

export function coerceUiSessionStatus(status: string): SessionStatus {
  if (
    (
      [
        "idle",
        "running",
        "needs_approval",
        "suspended",
        "failed",
        "done",
      ] as SessionStatus[]
    ).includes(status as SessionStatus)
  ) {
    return status as SessionStatus;
  }
  return "idle";
}

export function mergeTranscriptItems(
  prev: TranscriptItem[],
  incoming: TranscriptItem[],
): TranscriptItem[] {
  if (incoming.length === 0) return prev;
  const byId = new Map(prev.map((it) => [it.id, it]));
  const byMessage = new Map<string, number>();
  prev.forEach((it, i) => {
    if (it.messageId && (it.kind === "assistant" || it.kind === "user")) {
      byMessage.set(`${it.kind}:${it.messageId}`, i);
    }
  });
  const out = [...prev];
  let mutated = false;
  for (const item of incoming) {
    if (byId.has(item.id)) {
      const idx = out.findIndex((x) => x.id === item.id);
      if (idx >= 0) {
        const cur = out[idx]!;
        // Preserve object identity when wire payload is unchanged so
        // memoized TranscriptItemView rows skip re-render on quiet polls.
        if (!transcriptItemEqual(cur, item)) {
          out[idx] = item;
          mutated = true;
        }
      }
      continue;
    }
    // Streaming text: same message_id → replace/extend last chunk.
    if (
      item.messageId &&
      (item.kind === "assistant" || item.kind === "user" || item.kind === "reasoning")
    ) {
      const key = `${item.kind}:${item.messageId}`;
      const idx = byMessage.get(key);
      if (idx !== undefined && out[idx]) {
        const cur = out[idx]!;
        const nextText =
          item.text.length >= cur.text.length
            ? item.text
            : cur.text + item.text;
        const nextSeq = Math.max(cur.seq, item.seq);
        const nextTs = item.tsMs || cur.tsMs;
        if (
          nextText === cur.text &&
          nextSeq === cur.seq &&
          nextTs === cur.tsMs
        ) {
          continue;
        }
        out[idx] = {
          ...cur,
          text: nextText,
          seq: nextSeq,
          tsMs: nextTs,
        };
        mutated = true;
        continue;
      }
      byMessage.set(key, out.length);
    }
    // Approval: upsert by requestId.
    if (item.kind === "approval" && item.requestId) {
      const idx = out.findIndex(
        (x) => x.kind === "approval" && x.requestId === item.requestId,
      );
      if (idx >= 0) {
        const cur = out[idx]!;
        if (!transcriptItemEqual(cur, item)) {
          out[idx] = item;
          mutated = true;
        }
        continue;
      }
    }
    byId.set(item.id, item);
    out.push(item);
    mutated = true;
  }
  return mutated ? out : prev;
}

export function bumpStatus(
  prev: ResourceFetchStatus | undefined,
  quiet: boolean,
): { next: ResourceFetchStatus; generation: number } {
  const generation = (prev?.generation ?? 0) + 1;
  const phase: ResourceFetchPhase =
    quiet && prev?.phase === "ready" ? "ready" : "loading";
  return {
    generation,
    next: { phase, generation, error: undefined },
  };
}

export function toUiProject(p: DaemonProject): Project {
  return {
    id: p.id,
    name: p.name,
    workspacePath: p.workspacePath,
    conversationCount: p.conversationCount,
    runningAgents: p.runningAgents,
    needsAttention: p.needsAttention,
    updatedAtMs: p.updatedAtMs ?? 0,
    // Recomputed by patchProjectAggregates once conversations load.
    hasUnread: false,
    lastAttentionMs: 0,
  };
}

export function normalizeDaemonConversation(
  raw: DaemonConversation,
  fallbackProjectId?: string,
): DaemonConversation {
  const row = raw as DaemonConversation & {
    project_id?: string;
    conversation_id?: string;
    message_count?: number;
    updated_at_ms?: number;
    agent_session_count?: number;
  };
  const id = row.id || row.conversation_id || "";
  const projectId =
    row.projectId || row.project_id || fallbackProjectId || "";
  return {
    ...row,
    id,
    projectId,
    messageCount: row.messageCount ?? row.message_count ?? 0,
    updatedAtMs: row.updatedAtMs ?? row.updated_at_ms ?? 0,
    agentSessionCount: row.agentSessionCount ?? row.agent_session_count ?? 0,
  };
}

export function toUiConversation(
  c: DaemonConversation,
  readMessageCountById: Record<string, number>,
  activeConversationId: string | null,
  fallbackProjectId?: string,
): Conversation {
  const row = normalizeDaemonConversation(c, fallbackProjectId);
  const progress = parseProgress(row.progress);
  const priority = parsePriority(row.priority);
  const runningCount = row.runningCount ?? 0;
  const approvalCount = row.approvalCount ?? 0;
  // First time we see a conversation: baseline as read. Later growth = unread.
  // Active conversation never shows unread for its own messages.
  const baseline = readMessageCountById[row.id];
  const unread =
    row.id === activeConversationId
      ? 0
      : baseline === undefined
        ? 0
        : Math.max(0, row.messageCount - baseline);
  return {
    id: row.id,
    projectId: row.projectId,
    title: row.title,
    preview: row.preview || "No messages yet",
    // Prefer local relative formatting from ms (Rust string may be UTC-ish).
    updatedAt: row.updatedAtMs
      ? formatRelative(row.updatedAtMs)
      : row.updatedAt,
    updatedAtMs: row.updatedAtMs ?? 0,
    messageCount: row.messageCount,
    unread: unread > 0 ? unread : undefined,
    boardColumn: deriveBoardColumn({
      progress,
      runningCount,
      approvalCount,
    }),
    agentSessionCount: row.agentSessionCount,
    runningCount,
    approvalCount,
    priority,
    progress,
    branch: row.branch ?? undefined,
    worktree: row.worktree ?? undefined,
  };
}

export function patchLocalConversation(
  list: Conversation[],
  conversationId: string,
  patch: Partial<Conversation>,
): Conversation[] {
  return list.map((c) => {
    if (c.id !== conversationId) return c;
    const next = { ...c, ...patch };
    next.boardColumn = deriveBoardColumn({
      progress: next.progress ?? "todo",
      runningCount: next.runningCount,
      approvalCount: next.approvalCount,
    });
    return next;
  });
}

export function toUiMessage(m: DaemonMessage): TimelineMessage {
  const role =
    m.role === "user" || m.role === "agent" || m.role === "system"
      ? m.role
      : m.agent
        ? "agent"
        : "user";
  return {
    id: m.id,
    messageSeq: m.messageSeq,
    role,
    agent: (m.agent as AgentRuntime | null) ?? undefined,
    sessionId: m.sessionId ?? undefined,
    body: m.body,
    // Format in the browser with the user's local timezone.
    time: m.createdAtMs ? formatLocalClock(m.createdAtMs) : m.time,
    createdAtMs: m.createdAtMs,
    kind:
      m.kind === "approval" || m.kind === "tool_summary" ? m.kind : "text",
    replyToMessageId: m.replyToMessageId ?? undefined,
    delegationId: m.delegationId ?? undefined,
  };
}

export function toUiSession(s: DaemonSession): ProjectSession {
  const status = (
    [
      "idle",
      "running",
      "needs_approval",
      "suspended",
      "failed",
      "done",
    ] as SessionStatus[]
  ).includes(s.status as SessionStatus)
    ? (s.status as SessionStatus)
    : "idle";
  return {
    id: s.id,
    conversationId: s.conversationId,
    conversationTitle: s.conversationTitle ?? undefined,
    agent: (s.agent as AgentRuntime) || "codex",
    shortId: s.shortId,
    status,
    model: s.model,
    parentId: s.parentId ?? undefined,
    summary: s.summary,
    needsContinue: Boolean(s.needsContinue),
    firstTsMs: s.firstTsMs,
    lastTsMs: s.lastTsMs,
    messageCount: s.messageCount,
  };
}

export function mockBundle(): Pick<
  WorkspaceState,
  | "source"
  | "projects"
  | "conversations"
  | "messagesByConversation"
  | "sessionsByConversation"
  | "sessionsById"
  | "projectSessionsByProject"
  | "attentionSessions"
  | "attentionStatus"
> {
  const messagesByConversation: Record<string, TimelineMessage[]> = {
    ...mockTimeline,
  };
  const sessionsByConversation: Record<string, ProjectSession[]> = {};
  const sessionsById: Record<string, SessionEntity> = {};
  const projectSessionsByProject: Record<string, ProjectSession[]> = {};
  for (const s of mockSessions) {
    const list = sessionsByConversation[s.conversationId] ?? [];
    list.push(s);
    sessionsByConversation[s.conversationId] = list;
    const entity = mergeSessionEntity(undefined, {
      id: s.id,
      conversationId: s.conversationId,
      agent: s.agent,
      shortId: s.shortId,
      status: s.status,
      model: s.model,
      parentId: s.parentId,
      summary: s.summary,
      needsContinue: s.needsContinue,
    }, {
      pendingApproval: s.status === "needs_approval",
    });
    sessionsById[s.id] = entity;
    const projId =
      mockConversations.find((c) => c.id === s.conversationId)?.projectId ??
      "mock";
    const pList = projectSessionsByProject[projId] ?? [];
    pList.push(projectSessionFromEntity(entity) as ProjectSession);
    projectSessionsByProject[projId] = pList;
  }
  const attentionSessions = Object.values(sessionsById)
    .filter(entityNeedsAttention)
    .map((e) => projectSessionFromEntity(e) as ProjectSession);
  return {
    source: "mock",
    projects: mockProjects,
    conversations: mockConversations,
    messagesByConversation,
    sessionsByConversation,
    sessionsById,
    projectSessionsByProject,
    attentionSessions,
    attentionStatus: { phase: "ready", generation: 1 },
  };
}

export const idleStatus = (): ResourceFetchStatus => ({
  phase: "idle",
  generation: 0,
});

export const emptyWorkspace = {
  projects: [] as Project[],
  conversations: [] as Conversation[],
  conversationsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  messagesByConversation: {} as Record<string, TimelineMessage[]>,
  messageHistoryByConversation: {} as Record<string, MessageHistoryMeta>,
  sessionsByConversation: {} as Record<string, ProjectSession[]>,
  timelineStatusByConversation: {} as Record<string, ResourceFetchStatus>,
  inspectorStatusByConversation: {} as Record<string, ResourceFetchStatus>,
  timelineDirtyByConversation: {} as Record<string, boolean>,
  projectSessionsByProject: {} as Record<string, ProjectSession[]>,
  projectSessionsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  sessionsById: {} as Record<string, SessionEntity>,
  transcriptsBySession: {} as Record<string, TranscriptItem[]>,
  transcriptStatusBySession: {} as Record<string, ResourceFetchStatus>,
  transcriptHistoryBySession: {} as Record<string, TranscriptHistoryMeta>,
  attentionSessions: [] as ProjectSession[],
  attentionStatus: idleStatus(),
  clisStatus: idleStatus(),
};

export function patchProjectAggregates(
  projects: Project[],
  projectId: string,
  conversations: Conversation[],
): Project[] {
  const forProject = conversations.filter((c) => c.projectId === projectId);
  const needsAttention = forProject.reduce(
    (sum, c) => sum + (c.unread ?? 0) + (c.approvalCount ?? 0),
    0,
  );
  const runningAgents = forProject.reduce(
    (sum, c) => sum + (c.runningCount ?? 0),
    0,
  );
  const hasUnread = forProject.some(
    (c) => (c.unread ?? 0) > 0 || (c.approvalCount ?? 0) > 0,
  );
  const lastAttentionMs = forProject
    .filter((c) => (c.unread ?? 0) > 0 || (c.approvalCount ?? 0) > 0)
    .reduce((m, c) => Math.max(m, c.updatedAtMs ?? 0), 0);
  return projects.map((p) =>
    p.id === projectId
      ? {
          ...p,
          conversationCount: forProject.length,
          needsAttention,
          runningAgents,
          hasUnread,
          lastAttentionMs,
        }
      : p,
  );
}
