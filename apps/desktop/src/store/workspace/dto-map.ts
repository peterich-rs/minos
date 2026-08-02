/**
 * Daemon DTO → UI model mapping and small list patches (pure).
 */
import type {
  Conversation,
  Project,
  TimelineMessage,
  AgentRuntime,
  SessionStatus,
} from "@/shared/lib/mock-data";
import {
  type DaemonConversation,
  type DaemonMessage,
  type DaemonProject,
  type DaemonSession,
} from "@/shared/lib/daemon";
import {
  deriveBoardColumn,
  parsePriority,
  parseProgress,
} from "@/shared/lib/conversation-meta";
import { formatLocalClock, formatRelative } from "@/shared/lib/time";
import type {
  ProjectSession,
  ResourceFetchPhase,
  ResourceFetchStatus,
} from "./types";
import {
  normalizeGitActivity,
  timelineKindForMessage,
} from "./git-activity-map";

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

/**
 * Synthetic host-only project created by legacy agent start without Hub
 * conversation_id (`ensure_workspace_conversation`). Hide from the main
 * project rail so Hub collab does not appear as a second "Direct agent
 * sessions" project.
 */
export function isSyntheticDirectAgentProject(p: {
  id: string;
  name: string;
}): boolean {
  const name = p.name.trim();
  const id = p.id.trim();
  return (
    name === "Direct agent sessions" &&
    (id.startsWith("workspace-") || id.startsWith("project-"))
  );
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

/** Map daemon projects and drop synthetic Direct-agent shells. */
export function toUiProjects(projects: DaemonProject[]): Project[] {
  return projects
    .filter((p) => !isSyntheticDirectAgentProject(p))
    .map(toUiProject);
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
    participatingAgents: row.participatingAgents ?? [],
    runningCount,
    approvalCount,
    priority,
    progress,
    branch: row.branch ?? undefined,
    worktree: row.worktree ?? undefined,
    gitMode: row.gitMode ?? undefined,
    gitDirty: row.gitDirty ?? undefined,
    gitHead: row.gitHead ?? undefined,
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
  const gitActivity = normalizeGitActivity(m.gitActivity ?? null);
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
    kind: timelineKindForMessage(m.kind, gitActivity),
    replyToMessageId: m.replyToMessageId ?? undefined,
    delegationId: m.delegationId ?? undefined,
    gitActivity,
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
