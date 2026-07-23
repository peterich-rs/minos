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
import { demoteResolvedApprovalItems } from "@/shared/lib/session-status";
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

/** Kinds that close an open text/reasoning stream segment. */
function isTranscriptSegmentBreaker(kind: string): boolean {
  return (
    kind === "tool" ||
    kind === "tool_result" ||
    kind === "tool_error" ||
    kind === "subagent" ||
    kind === "status" ||
    kind === "approval" ||
    kind === "question" ||
    kind === "error"
  );
}

function isToolLifecycleKind(kind: string): boolean {
  return kind === "tool" || kind === "tool_result" || kind === "tool_error";
}

/**
 * Merge same-id tool cards across live frames.
 * Never demote tool_result/tool_error back to open `tool` (title refine frames).
 */
export function mergeToolLifecycleItems(
  cur: TranscriptItem,
  incoming: TranscriptItem,
): TranscriptItem {
  const curDone = cur.kind === "tool_result" || cur.kind === "tool_error";
  const inDone =
    incoming.kind === "tool_result" || incoming.kind === "tool_error";
  let kind = cur.kind;
  if (inDone) {
    // Prefer error over success if either side failed.
    kind =
      cur.kind === "tool_error" || incoming.kind === "tool_error"
        ? "tool_error"
        : "tool_result";
  } else if (!curDone && incoming.kind === "tool") {
    kind = "tool";
  }
  // else curDone && incoming is tool → keep completed kind

  const preferIncomingDetail =
    inDone &&
    (incoming.detail?.length ?? 0) >= (cur.detail?.length ?? 0);
  const detail = preferIncomingDetail
    ? (incoming.detail ?? cur.detail)
    : (cur.detail ?? incoming.detail);

  const title =
    incoming.title && incoming.title.trim()
      ? incoming.title
      : cur.title;
  // Prefer path-like targets over empty/tool-name fallbacks.
  const text =
    incoming.text &&
    incoming.text.trim() &&
    !(curDone && !inDone && incoming.text.length < cur.text.length)
      ? incoming.text
      : cur.text || incoming.text;

  return {
    ...cur,
    ...incoming,
    id: cur.id,
    kind,
    title,
    text,
    detail: detail ?? null,
    requestId: incoming.requestId || cur.requestId,
    messageId: incoming.messageId || cur.messageId,
    seq: Math.max(cur.seq, incoming.seq),
    tsMs: Math.max(cur.tsMs, incoming.tsMs),
  };
}

/** Collapse same-id rows (legacy Place twin) preferring completed tools. */
export function dedupeTranscriptItemsById(
  items: TranscriptItem[],
): TranscriptItem[] {
  const byId = new Map<string, TranscriptItem>();
  const order: string[] = [];
  for (const it of items) {
    const prev = byId.get(it.id);
    if (!prev) {
      byId.set(it.id, it);
      order.push(it.id);
      continue;
    }
    if (isToolLifecycleKind(prev.kind) && isToolLifecycleKind(it.kind)) {
      byId.set(it.id, mergeToolLifecycleItems(prev, it));
    } else if (it.seq >= prev.seq) {
      byId.set(it.id, it);
    }
  }
  return order.map((id) => byId.get(id)!);
}

/**
 * Index of the open streamable bubble for kind+messageId, or -1.
 * Open = last matching row with no segment-breaker after it (tools/subagent…).
 * Prevents live frames from rewriting frozen mid-timeline narration.
 */
function findOpenStreamSlot(
  out: TranscriptItem[],
  kind: string,
  messageId: string,
): number {
  for (let i = out.length - 1; i >= 0; i--) {
    const it = out[i]!;
    if (it.kind !== kind || it.messageId !== messageId) continue;
    const closed = out
      .slice(i + 1)
      .some((x) => isTranscriptSegmentBreaker(x.kind));
    return closed ? -1 : i;
  }
  return -1;
}

export function mergeTranscriptItems(
  prev: TranscriptItem[],
  incoming: TranscriptItem[],
): TranscriptItem[] {
  // Empty incoming still collapses legacy multi-row subagent cards.
  if (incoming.length === 0) {
    return demoteResolvedApprovalItems(collapseDuplicateSubagentCards(prev));
  }
  const byId = new Map(prev.map((it) => [it.id, it]));
  const out = [...prev];
  let mutated = false;
  for (const item of incoming) {
    if (byId.has(item.id)) {
      const idx = out.findIndex((x) => x.id === item.id);
      if (idx >= 0) {
        const cur = out[idx]!;
        // Live ingest frames are assembled per-frame: each TextDelta becomes a
        // full row with *only that chunk*. Naïve by-id replace keeps the latest
        // token and drops prior text. Prefer a longer/prefix snapshot; else
        // append while this streamable bubble is still the timeline tail.
        if (
          (item.kind === "assistant" ||
            item.kind === "user" ||
            item.kind === "reasoning" ||
            item.kind === "text") &&
          item.kind === cur.kind
        ) {
          const isTail = idx === out.length - 1;
          let nextText = cur.text;
          if (item.text !== cur.text) {
            if (
              item.text.length >= cur.text.length &&
              (cur.text.length === 0 || item.text.startsWith(cur.text))
            ) {
              nextText = item.text;
            } else if (isTail) {
              nextText = cur.text + item.text;
            }
            // Non-tail: freeze mid-timeline narration (ignore shorter deltas).
          }
          const next: TranscriptItem = {
            ...cur,
            ...item,
            id: cur.id,
            text: nextText,
            seq: Math.max(cur.seq, item.seq),
            tsMs: item.tsMs || cur.tsMs,
          };
          if (!transcriptItemEqual(cur, next)) {
            out[idx] = next;
            byId.set(cur.id, next);
            mutated = true;
          }
          continue;
        }
        // Tool lifecycle: progressive complete + late title Place must not
        // demote tool_result → tool (would show "edit in flight" while Idle).
        if (isToolLifecycleKind(cur.kind) && isToolLifecycleKind(item.kind)) {
          const next = mergeToolLifecycleItems(cur, item);
          if (!transcriptItemEqual(cur, next)) {
            out[idx] = next;
            byId.set(cur.id, next);
            mutated = true;
          }
          continue;
        }
        // Preserve object identity when wire payload is unchanged so
        // memoized TranscriptItemView rows skip re-render on quiet polls.
        if (!transcriptItemEqual(cur, item)) {
          out[idx] = item;
          byId.set(item.id, item);
          mutated = true;
        }
      }
      continue;
    }
    // Subagent: one card per tool_call / sub_session — always in-place upsert.
    if (item.kind === "subagent") {
      let idx = out.findIndex((x) => subagentCardsMatch(x, item));
      // Live frames: "Running" may only have tool requestId; completed may only
      // have sub_session messageId — still one card if a single orphan Running.
      if (idx < 0 && item.messageId) {
        const orphans = out
          .map((x, i) => ({ x, i }))
          .filter(
            ({ x }) =>
              x.kind === "subagent" &&
              !x.messageId &&
              /\bRunning\b/i.test(x.text),
          );
        if (orphans.length === 1) idx = orphans[0]!.i;
      }
      if (idx >= 0) {
        const cur = out[idx]!;
        const merged = mergeSubagentCard(cur, item);
        if (!transcriptItemEqual(cur, merged)) {
          out[idx] = merged;
          byId.set(merged.id, merged);
          if (cur.id !== merged.id) byId.delete(cur.id);
          mutated = true;
        }
        continue;
      }
    }
    // Streaming text: only extend an *open* tail segment (not above tools).
    if (
      item.messageId &&
      (item.kind === "assistant" ||
        item.kind === "user" ||
        item.kind === "reasoning")
    ) {
      const idx = findOpenStreamSlot(out, item.kind, item.messageId);
      if (idx >= 0 && out[idx]) {
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
        byId.set(cur.id, out[idx]!);
        mutated = true;
        continue;
      }
      // Closed segment or no prior row → append (new part / post-tool text).
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
  // Collapse any subagent duplicates that slipped in (different ids, same session/tool).
  const collapsed = collapseDuplicateSubagentCards(out);
  // History keeps raw approval/request frames after the user already decided;
  // demote cards that are followed by later agent/user progress.
  const finalized = demoteResolvedApprovalItems(collapsed);
  if (finalized !== collapsed) return finalized;
  if (collapsed !== out) return collapsed;
  return mutated ? out : prev;
}

/** Short id painted in the header (`#ses_072f`) for fuzzy match across frames. */
function subagentDisplayKey(it: TranscriptItem): string | null {
  if (it.messageId) return it.messageId.slice(0, 12);
  if (it.requestId) return `tool:${it.requestId}`;
  const m = it.text.match(/#([A-Za-z0-9_]+)/);
  return m?.[1] ?? null;
}

/** True when two subagent rows refer to the same task/session. */
function subagentCardsMatch(a: TranscriptItem, b: TranscriptItem): boolean {
  if (a.kind !== "subagent" || b.kind !== "subagent") return false;
  if (a.id === b.id) return true;
  if (a.requestId && b.requestId && a.requestId === b.requestId) return true;
  if (a.messageId && b.messageId && a.messageId === b.messageId) return true;
  // Prefer matching a session-scoped id against tool-scoped after spawn.
  if (a.messageId && (b.id === `subagent:${a.messageId}` || b.id === `subagent:ses:${a.messageId}`))
    return true;
  if (b.messageId && (a.id === `subagent:${b.messageId}` || a.id === `subagent:ses:${b.messageId}`))
    return true;
  if (a.requestId && b.id === `subagent:tool:${a.requestId}`) return true;
  if (b.requestId && a.id === `subagent:tool:${b.requestId}`) return true;
  // Same painted #short id (legacy duplicates with different internal ids).
  const ka = subagentDisplayKey(a);
  const kb = subagentDisplayKey(b);
  if (ka && kb && ka === kb) return true;
  return false;
}

/** Prefer session id, richer detail, non-placeholder agent, later status. */
function mergeSubagentCard(
  cur: TranscriptItem,
  incoming: TranscriptItem,
): TranscriptItem {
  const messageId = incoming.messageId || cur.messageId || null;
  const requestId = incoming.requestId || cur.requestId || null;
  const id = messageId
    ? `subagent:${messageId}`
    : requestId
      ? `subagent:tool:${requestId}`
      : cur.id;
  const title =
    incoming.title && incoming.title !== "subagent"
      ? incoming.title
      : cur.title && cur.title !== "subagent"
        ? cur.title
        : (incoming.title ?? cur.title ?? "opencode");
  // Prefer completed/failed over running when both present.
  const preferIncoming =
    incoming.seq >= cur.seq ||
    (/\b(completed|failed|interrupted)\b/i.test(incoming.text) &&
      /\bRunning\b/i.test(cur.text));
  return {
    ...cur,
    id,
    kind: "subagent",
    text: preferIncoming ? incoming.text : cur.text,
    title,
    detail: incoming.detail || cur.detail,
    messageId,
    requestId,
    seq: Math.max(cur.seq, incoming.seq),
    tsMs: Math.max(cur.tsMs, incoming.tsMs),
  };
}

function collapseDuplicateSubagentCards(
  items: TranscriptItem[],
): TranscriptItem[] {
  const subIdx: number[] = [];
  items.forEach((it, i) => {
    if (it.kind === "subagent") subIdx.push(i);
  });
  if (subIdx.length <= 1) return items;

  const next = items.slice();
  const drop = new Set<number>();
  for (let a = 0; a < subIdx.length; a++) {
    for (let b = a + 1; b < subIdx.length; b++) {
      const ia = subIdx[a]!;
      const ib = subIdx[b]!;
      if (drop.has(ia) || drop.has(ib)) continue;
      if (!subagentCardsMatch(next[ia]!, next[ib]!)) continue;
      // Keep earlier position; fold later status into it.
      next[ia] = mergeSubagentCard(next[ia]!, next[ib]!);
      drop.add(ib);
    }
  }
  if (drop.size === 0) return items;
  return next.filter((_, i) => !drop.has(i));
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
