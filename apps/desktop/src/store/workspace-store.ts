import { create } from "zustand";
import {
  conversations as mockConversations,
  projects as mockProjects,
  timelineByConversation as mockTimeline,
  agentSessions as mockSessions,
  type Conversation,
  type ConversationProgress,
  type Project,
  type TimelineMessage,
  type AgentSession,
  type AgentRuntime,
  type SessionStatus,
} from "@/lib/mock-data";
import {
  daemonApi,
  isTauriRuntime,
  type DaemonConnection,
  type DaemonConversation,
  type DaemonConversationEvent,
  type DaemonIngestEvent,
  type DaemonManagerEvent,
  type DaemonMessage,
  type DaemonProject,
  type DaemonSession,
  type TranscriptItem,
} from "@/lib/daemon";
import { startDaemonEventBridge } from "@/lib/daemon-events";
import {
  deriveBoardColumn,
  nextPriority,
  nextProgress,
  parsePriority,
  parseProgress,
  progressForBoardColumn,
} from "@/lib/conversation-meta";
import { parseAgentRouting, type KnownAgent } from "@/lib/agent-route";
import {
  nextSessionStatusAfterTranscript,
  transcriptHasPendingApproval,
  withDerivedSessionStatuses,
  type ApprovalStatusPolicy,
} from "@/lib/session-status";
import { formatLocalClock, formatRelative } from "@/lib/time";
import type {
  ConversationBoardColumn,
} from "@/lib/mock-data";

const KNOWN_AGENTS_FALLBACK: {
  agent: string;
  installed: boolean;
  status: string;
}[] = [
  { agent: "codex", installed: true, status: "ok" },
  { agent: "claude", installed: true, status: "ok" },
  { agent: "gemini", installed: true, status: "ok" },
  { agent: "opencode", installed: false, status: "missing" },
  { agent: "grok", installed: true, status: "ok" },
];

export type DataSource = "mock" | "daemon";

/** Shared per-resource fetch lifecycle (generation guards stale commits). */
export type ResourceFetchPhase = "idle" | "loading" | "ready" | "error";

export type ResourceFetchStatus = {
  phase: ResourceFetchPhase;
  error?: string;
  generation: number;
};

/** @deprecated alias — use ResourceFetchStatus */
export type ConversationDetailPhase = ResourceFetchPhase;
export type ConversationDetailStatus = ResourceFetchStatus;

type WorkspaceState = {
  /** True while connecting / starting managed daemon (no main UI yet). */
  booting: boolean;
  bootPhase: string;
  bootProgress: number;
  /**
   * Increments on each successful daemon bootstrap.
   * Views depend on this so conversation/session lists re-init after boot
   * clears workspace caches (StrictMode / reconnect).
   */
  bootEpoch: number;
  /**
   * True when Tauri push subscriptions (daemon://*) are active.
   * Live UI should prefer events over quiet poll intervals.
   */
  livePush: boolean;
  source: DataSource;
  connection: DaemonConnection | null;
  loading: boolean;
  /** Boot / connection failures only (not per-resource). */
  error: string | null;
  /** Transient action errors (send, create, mutate) for banners. */
  actionError: string | null;
  projects: Project[];
  conversations: Conversation[];
  /** Per-project conversation list fetch status. */
  conversationsStatusByProject: Record<string, ResourceFetchStatus>;
  messagesByConversation: Record<string, TimelineMessage[]>;
  sessionsByConversation: Record<string, ProjectSession[]>;
  /** Per-id load status for conversation timeline/detail. */
  detailStatusByConversation: Record<string, ResourceFetchStatus>;
  /** Project-scoped aggregate sessions (Sessions tab). Keyed by projectId. */
  projectSessionsByProject: Record<string, ProjectSession[]>;
  projectSessionsStatusByProject: Record<string, ResourceFetchStatus>;
  /**
   * @deprecated Prefer projectSessionsByProject[projectId].
   * Mirror of the active project's sessions for older call sites.
   */
  projectSessions: ProjectSession[];
  transcriptsByThread: Record<string, TranscriptItem[]>;
  transcriptStatusByThread: Record<string, ResourceFetchStatus>;
  /** Cross-project sessions needing attention (Attention tab). */
  attentionSessions: ProjectSession[];
  attentionStatus: ResourceFetchStatus;
  clis: { agent: string; installed: boolean; status: string }[];
  clisStatus: ResourceFetchStatus;
  /** messageCount snapshot when the user last opened the conversation. */
  readMessageCountById: Record<string, number>;
  /** Conversation currently open in the timeline (unread forced to 0). */
  focusedConversationId: string | null;

  bootstrap: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  loadConversations: (
    projectId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  /**
   * Load messages + agent sessions for one conversation.
   * `quiet`: background refresh (poll) — keep prior data, skip loading flash.
   */
  loadConversationDetail: (
    conversationId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  loadProjectSessions: (
    projectId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  loadTranscript: (
    threadId: string,
    opts?: {
      append?: boolean;
      tailWindow?: number;
      quiet?: boolean;
      /**
       * elevate-only: conversation quiet-poll peeks — never demote needs_approval
       * (tail window can miss pending cards and thrash the UI).
       * sync (default): full/open/append loads may clear elevation.
       */
      approvalStatusPolicy?: ApprovalStatusPolicy;
    },
  ) => Promise<void>;
  /** Load attention queue across all projects (approvals / failed / suspended). */
  loadAttentionSessions: (opts?: { quiet?: boolean }) => Promise<void>;
  resolveApproval: (
    threadId: string,
    requestId: string,
    decision: "approve" | "revise" | "abandon",
  ) => Promise<void>;
  /** Mark conversation messages as read (clears unread badge). */
  markConversationRead: (conversationId: string) => void;
  clearActionError: () => void;
  sendMessage: (conversationId: string, body: string) => Promise<void>;
  createConversation: (projectId: string, title: string) => Promise<string | null>;
  updateConversationTitle: (
    conversationId: string,
    title: string,
  ) => Promise<void>;
  cycleConversationPriority: (conversationId: string) => Promise<void>;
  cycleConversationProgress: (conversationId: string) => Promise<void>;
  setConversationProgress: (
    conversationId: string,
    progress: ConversationProgress,
  ) => Promise<void>;
  /** Board move: maps column → progress (needs_you → in_progress). */
  moveConversationToBoardColumn: (
    conversationId: string,
    column: ConversationBoardColumn,
  ) => Promise<void>;
  createProject: (workspacePath: string) => Promise<string>;
  loadClis: (opts?: { quiet?: boolean }) => Promise<void>;
  /** Apply live push frames from the Tauri daemon event bridge. */
  applyIngestEvent: (ev: DaemonIngestEvent) => void;
  applyManagerEvent: (ev: DaemonManagerEvent) => void;
  applyConversationEvent: (ev: DaemonConversationEvent) => void;
};

function mapSessionStatusInLists(
  s: {
    sessionsByConversation: Record<string, ProjectSession[]>;
    projectSessionsByProject: Record<string, ProjectSession[]>;
    projectSessions: ProjectSession[];
  },
  threadId: string,
  map: (sess: ProjectSession) => ProjectSession,
): Pick<
  WorkspaceState,
  "sessionsByConversation" | "projectSessionsByProject" | "projectSessions"
> {
  const sessionsByConversation = { ...s.sessionsByConversation };
  for (const [cid, list] of Object.entries(sessionsByConversation)) {
    sessionsByConversation[cid] = list.map((sess) =>
      sess.id === threadId ? map(sess) : sess,
    );
  }
  const projectSessionsByProject = { ...s.projectSessionsByProject };
  for (const [pid, list] of Object.entries(projectSessionsByProject)) {
    projectSessionsByProject[pid] = list.map((sess) =>
      sess.id === threadId ? map(sess) : sess,
    );
  }
  const projectSessions = s.projectSessions.map((sess) =>
    sess.id === threadId ? map(sess) : sess,
  );
  return { sessionsByConversation, projectSessionsByProject, projectSessions };
}

function coerceUiSessionStatus(status: string): SessionStatus {
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

/** Merge live transcript items into a cached thread list (id-stable). */
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
  for (const item of incoming) {
    if (byId.has(item.id)) {
      const idx = out.findIndex((x) => x.id === item.id);
      if (idx >= 0) out[idx] = item;
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
        out[idx] = {
          ...cur,
          text:
            item.text.length >= cur.text.length
              ? item.text
              : cur.text + item.text,
          seq: Math.max(cur.seq, item.seq),
          tsMs: item.tsMs || cur.tsMs,
        };
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
        out[idx] = item;
        continue;
      }
    }
    byId.set(item.id, item);
    out.push(item);
  }
  return out;
}

function bumpStatus(
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

export type ProjectSession = AgentSession & {
  conversationTitle?: string;
  firstTsMs?: number;
  lastTsMs?: number;
  /** Thread last_seq — used to seek transcript tail. */
  messageCount?: number;
};

/** In-flight bootstrap so React StrictMode double-mount cannot wipe loads. */
let bootstrapInFlight: Promise<void> | null = null;

function toUiProject(p: DaemonProject): Project {
  return {
    id: p.id,
    name: p.name,
    workspacePath: p.workspacePath,
    conversationCount: p.conversationCount,
    runningAgents: p.runningAgents,
    needsAttention: p.needsAttention,
  };
}

/**
 * Normalize a daemon conversation row for UI mapping.
 * Stamps `fallbackProjectId` when the wire object omits projectId (or only
 * has snake_case), so list filters by project never drop real rows.
 */
function normalizeDaemonConversation(
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

function toUiConversation(
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

function patchLocalConversation(
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

function toUiMessage(m: DaemonMessage): TimelineMessage {
  const role =
    m.role === "user" || m.role === "agent" || m.role === "system"
      ? m.role
      : m.agent
        ? "agent"
        : "user";
  return {
    id: m.id,
    role,
    agent: (m.agent as AgentRuntime | null) ?? undefined,
    sessionId: m.sessionId ?? undefined,
    body: m.body,
    // Format in the browser with the user's local timezone.
    time: m.createdAtMs ? formatLocalClock(m.createdAtMs) : m.time,
    kind:
      m.kind === "approval" || m.kind === "tool_summary" ? m.kind : "text",
  };
}

function toUiSession(s: DaemonSession): ProjectSession {
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

function mockBundle(): Pick<
  WorkspaceState,
  | "source"
  | "projects"
  | "conversations"
  | "messagesByConversation"
  | "sessionsByConversation"
> {
  const messagesByConversation: Record<string, TimelineMessage[]> = {
    ...mockTimeline,
  };
  const sessionsByConversation: Record<string, AgentSession[]> = {};
  for (const s of mockSessions) {
    const list = sessionsByConversation[s.conversationId] ?? [];
    list.push(s);
    sessionsByConversation[s.conversationId] = list;
  }
  return {
    source: "mock",
    projects: mockProjects,
    conversations: mockConversations,
    messagesByConversation,
    sessionsByConversation,
  };
}

const idleStatus = (): ResourceFetchStatus => ({
  phase: "idle",
  generation: 0,
});

const emptyWorkspace = {
  projects: [] as Project[],
  conversations: [] as Conversation[],
  conversationsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  messagesByConversation: {} as Record<string, TimelineMessage[]>,
  sessionsByConversation: {} as Record<string, ProjectSession[]>,
  detailStatusByConversation: {} as Record<string, ResourceFetchStatus>,
  projectSessionsByProject: {} as Record<string, ProjectSession[]>,
  projectSessionsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  projectSessions: [] as ProjectSession[],
  transcriptsByThread: {} as Record<string, TranscriptItem[]>,
  transcriptStatusByThread: {} as Record<string, ResourceFetchStatus>,
  attentionSessions: [] as ProjectSession[],
  attentionStatus: idleStatus(),
  clisStatus: idleStatus(),
};

function patchProjectAggregates(
  projects: Project[],
  projectId: string,
  conversations: Conversation[],
): Project[] {
  const forProject = conversations.filter((c) => c.projectId === projectId);
  const needsAttention = forProject.reduce(
    (sum, c) => sum + (c.approvalCount ?? 0),
    0,
  );
  const runningAgents = forProject.reduce(
    (sum, c) => sum + (c.runningCount ?? 0),
    0,
  );
  return projects.map((p) =>
    p.id === projectId
      ? {
          ...p,
          conversationCount: forProject.length,
          needsAttention,
          runningAgents,
        }
      : p,
  );
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  booting: true,
  bootPhase: "Starting…",
  bootProgress: 5,
  bootEpoch: 0,
  livePush: false,
  source: "daemon",
  connection: null,
  loading: false,
  error: null,
  actionError: null,
  clis: KNOWN_AGENTS_FALLBACK,
  readMessageCountById: {},
  focusedConversationId: null,
  ...emptyWorkspace,

  bootstrap: async () => {
    // Single-flight: all concurrent callers (React StrictMode double mount)
    // await the same promise instead of wiping emptyWorkspace twice mid-load.
    if (bootstrapInFlight) {
      return bootstrapInFlight;
    }
    const alreadyReady =
      !get().booting &&
      get().connection?.connected &&
      get().source === "daemon" &&
      get().bootEpoch > 0;
    if (alreadyReady) {
      return;
    }

    bootstrapInFlight = (async () => {
      // Browser-only Vite: mock is intentional for UI work.
      if (!isTauriRuntime()) {
        set({
          ...mockBundle(),
          booting: false,
          bootPhase: "Ready",
          bootProgress: 100,
          bootEpoch: get().bootEpoch + 1,
          connection: null,
          error: null,
          actionError: null,
          loading: false,
          clis: KNOWN_AGENTS_FALLBACK,
          clisStatus: { phase: "ready", generation: 1 },
          attentionStatus: { phase: "ready", generation: 1 },
        });
        return;
      }

      set({
        booting: true,
        bootPhase: "Connecting to daemon…",
        bootProgress: 12,
        error: null,
        // Never show mock fixtures while booting in Tauri.
        ...emptyWorkspace,
        source: "daemon",
      });

      try {
        set({
          bootPhase: "Starting or discovering daemon…",
          bootProgress: 28,
        });
        const connection = await daemonApi.connect();

        if (!connection.connected) {
          set({
            booting: false,
            bootProgress: 100,
            bootPhase: "Daemon unavailable",
            connection,
            error: connection.error,
            source: "daemon",
            ...emptyWorkspace,
            clis: KNOWN_AGENTS_FALLBACK,
            loading: false,
          });
          return;
        }

        set({
          bootPhase: connection.managed
            ? "Managed daemon ready · loading projects…"
            : "Daemon online · loading projects…",
          bootProgress: 55,
          connection,
        });

        const projects = (await daemonApi.listProjects()).map(toUiProject);

        set({ bootPhase: "Loading agents…", bootProgress: 72 });
        let clis = KNOWN_AGENTS_FALLBACK;
        try {
          clis = (await daemonApi.listClis()).map((c) => ({
            agent: c.agent,
            installed: c.installed,
            status: c.status,
          }));
        } catch {
          /* keep fallback */
        }

        // Conversations load lazily when a project view mounts (not all projects).
        set({
          booting: false,
          bootPhase: "Ready",
          bootProgress: 100,
          bootEpoch: get().bootEpoch + 1,
          source: "daemon",
          connection,
          loading: false,
          error: null,
          actionError: null,
          focusedConversationId: null,
          readMessageCountById: {},
          ...emptyWorkspace,
          projects,
          clis,
          clisStatus: { phase: "ready", generation: 1 },
        });

        // Arm TUI-parity push subscriptions (ingest / manager / conversation).
        try {
          await startDaemonEventBridge({
            onIngest: (ev) => get().applyIngestEvent(ev),
            onManager: (ev) => get().applyManagerEvent(ev),
            onConversation: (ev) => get().applyConversationEvent(ev),
          });
          set({ livePush: true });
        } catch {
          set({ livePush: false });
        }
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        set({
          booting: false,
          bootPhase: "Failed",
          bootProgress: 100,
          source: "daemon",
          connection: {
            connected: false,
            endpoint: null,
            error: message,
            source: "error",
            managed: false,
          },
          error: message,
          actionError: null,
          loading: false,
          clis: KNOWN_AGENTS_FALLBACK,
          ...emptyWorkspace,
        });
      }
    })();

    try {
      await bootstrapInFlight;
    } finally {
      bootstrapInFlight = null;
    }
  },

  refreshProjects: async () => {
    if (get().source !== "daemon") return;
    try {
      const projects = (await daemonApi.listProjects()).map(toUiProject);
      // Preserve aggregates computed from conversation lists.
      const prev = get().projects;
      const merged = projects.map((p) => {
        const old = prev.find((x) => x.id === p.id);
        return old
          ? {
              ...p,
              needsAttention: old.needsAttention,
              runningAgents: old.runningAgents,
              conversationCount:
                old.conversationCount || p.conversationCount,
            }
          : p;
      });
      set({ projects: merged });
    } catch (e) {
      set({
        actionError: e instanceof Error ? e.message : String(e),
      });
    }
  },

  clearActionError: () => set({ actionError: null }),

  loadConversations: async (projectId, opts) => {
    if (get().source !== "daemon" || !projectId) return;
    const quiet = opts?.quiet === true;
    const prev = get().conversationsStatusByProject[projectId];
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () =>
      get().conversationsStatusByProject[projectId]?.generation !== generation;

    set((s) => ({
      conversationsStatusByProject: {
        ...s.conversationsStatusByProject,
        [projectId]: next,
      },
    }));

    try {
      const rows = await daemonApi.listConversations(projectId);
      if (isStale()) return;
      const normalized = rows.map((row) =>
        normalizeDaemonConversation(row, projectId),
      );
      const read = { ...get().readMessageCountById };
      // Baseline first sight so the list doesn't scream "unread" for history.
      for (const row of normalized) {
        if (row.id && read[row.id] === undefined) {
          read[row.id] = row.messageCount;
        }
      }
      const focused = get().focusedConversationId;
      if (focused) {
        const focusedRow = normalized.find((r) => r.id === focused);
        if (focusedRow) {
          read[focused] = focusedRow.messageCount;
        }
      }
      // Stamp query projectId so UI filters never drop rows after wire mapping.
      const list = normalized
        .filter((row) => Boolean(row.id))
        .map((row) => toUiConversation(row, read, focused, projectId));
      const others = get().conversations.filter(
        (c) => c.projectId !== projectId,
      );
      const conversations = [...others, ...list];
      set((s) => ({
        conversations,
        readMessageCountById: read,
        conversationsStatusByProject: {
          ...s.conversationsStatusByProject,
          [projectId]: { phase: "ready", generation },
        },
        projects: patchProjectAggregates(s.projects, projectId, conversations),
      }));
    } catch (e) {
      if (isStale()) return;
      const message = e instanceof Error ? e.message : String(e);
      set((s) => ({
        conversationsStatusByProject: {
          ...s.conversationsStatusByProject,
          [projectId]: { phase: "error", generation, error: message },
        },
      }));
    }
  },

  loadConversationDetail: async (conversationId, opts) => {
    if (get().source !== "daemon" || !conversationId) return;
    const quiet = opts?.quiet === true;
    const prev = get().detailStatusByConversation[conversationId];
    const generation = (prev?.generation ?? 0) + 1;
    const isStale = () =>
      get().detailStatusByConversation[conversationId]?.generation !==
      generation;

    set((s) => ({
      detailStatusByConversation: {
        ...s.detailStatusByConversation,
        [conversationId]: {
          phase: quiet && prev?.phase === "ready" ? "ready" : "loading",
          generation,
          error: undefined,
        },
      },
    }));

    try {
      const [messages, sessions] = await Promise.all([
        daemonApi.listMessages(conversationId),
        daemonApi.listSessions(conversationId),
      ]);
      if (isStale()) return;

      const daemonSessions = sessions.map(toUiSession);
      const prevStatuses = new Map(
        (get().sessionsByConversation[conversationId] ?? []).map((s) => [
          s.id,
          s.status,
        ]),
      );
      // Hold client-derived needs_approval across listSessions (daemon only
      // reports running while parked on permission/plan reverse-requests).
      const heldSessions = withDerivedSessionStatuses(
        daemonSessions,
        prevStatuses,
      );
      const uiMessages = messages.map(toUiMessage);
      set((s) => ({
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: uiMessages,
        },
        sessionsByConversation: {
          ...s.sessionsByConversation,
          [conversationId]: heldSessions,
        },
        detailStatusByConversation: {
          ...s.detailStatusByConversation,
          [conversationId]: { phase: "ready", generation },
        },
        error: null,
      }));

      // At most one top-level interrupted session auto-continues on open.
      // Skip on quiet poll to avoid double-continue races.
      if (!quiet) {
        const continueTarget = daemonSessions
          .filter((s) => !s.parentId && s.needsContinue && s.status !== "done")
          .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
        if (continueTarget) {
          try {
            await daemonApi.resumeThread(continueTarget.id, true);
          } catch {
            /* reattach/continue best-effort; send path can retry */
          }
        }
      }
      if (isStale()) return;

      // Grok plan/permission approvals keep thread state as `running` while
      // parking on a reverse-request. Peek the transcript tail so we can
      // surface needs_approval + pending approval cards.
      const active = daemonSessions.filter(
        (s) =>
          !s.parentId &&
          (s.status === "running" ||
            s.status === "suspended" ||
            prevStatuses.get(s.id) === "needs_approval"),
      );
      await Promise.all(
        active.map((s) =>
          get()
            .loadTranscript(s.id, {
              tailWindow: 120,
              quiet: true,
              // Never demote from a short tail peek — that thrash is the
              // needs_approval ↔ running flash on the banner/inspector.
              approvalStatusPolicy: "elevate-only",
            })
            .catch(() => {
              /* ignore tail peek errors */
            }),
        ),
      );
      if (isStale()) return;

      // Only publish *positive* pending signals from peeks. Absence is
      // ambiguous (window miss) — keep preserve via undefined.
      // Clear elevation only when daemon left `running` (deriveSessionStatus).
      const pendingById = new Map<string, boolean>();
      const transcripts = get().transcriptsByThread;
      for (const s of active) {
        if (s.id in transcripts) {
          const pending = transcriptHasPendingApproval(
            transcripts[s.id] ?? [],
          );
          if (pending) {
            pendingById.set(s.id, true);
          } else if (
            s.status === "idle" ||
            s.status === "done" ||
            s.status === "failed"
          ) {
            pendingById.set(s.id, false);
          }
        }
      }
      const finalSessions = withDerivedSessionStatuses(
        daemonSessions,
        prevStatuses,
        pendingById.size > 0 ? pendingById : undefined,
      );
      const runningCount = finalSessions.filter(
        (s) => s.status === "running",
      ).length;
      const approvalCount = finalSessions.filter(
        (s) => s.status === "needs_approval" || s.status === "suspended",
      ).length;
      set((s) => ({
        sessionsByConversation: {
          ...s.sessionsByConversation,
          [conversationId]: finalSessions,
        },
        conversations: patchLocalConversation(s.conversations, conversationId, {
          runningCount,
          approvalCount,
        }),
      }));
      // Opening the conversation clears unread message attention.
      if (!quiet) {
        get().markConversationRead(conversationId);
      }
    } catch (e) {
      if (isStale()) return;
      const message = e instanceof Error ? e.message : String(e);
      set((s) => ({
        detailStatusByConversation: {
          ...s.detailStatusByConversation,
          [conversationId]: {
            phase: "error",
            generation,
            error: message,
          },
        },
      }));
    }
  },

  loadProjectSessions: async (projectId, opts) => {
    if (get().source !== "daemon" || !projectId) return;
    const quiet = opts?.quiet === true;
    const prev = get().projectSessionsStatusByProject[projectId];
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () =>
      get().projectSessionsStatusByProject[projectId]?.generation !==
      generation;

    set((s) => ({
      projectSessionsStatusByProject: {
        ...s.projectSessionsStatusByProject,
        [projectId]: next,
      },
    }));

    try {
      const daemonSessions = (await daemonApi.listProjectSessions(projectId)).map(
        toUiSession,
      );
      if (isStale()) return;
      const prevList =
        get().projectSessionsByProject[projectId] ?? get().projectSessions;
      const prevStatuses = new Map(prevList.map((s) => [s.id, s.status]));
      const pendingById = new Map<string, boolean>();
      for (const s of daemonSessions) {
        const items = get().transcriptsByThread[s.id];
        if (items) {
          pendingById.set(s.id, transcriptHasPendingApproval(items));
        }
      }
      const sessions = withDerivedSessionStatuses(
        daemonSessions,
        prevStatuses,
        pendingById.size > 0 ? pendingById : undefined,
      );
      set((s) => ({
        projectSessionsByProject: {
          ...s.projectSessionsByProject,
          [projectId]: sessions,
        },
        // Keep legacy mirror in sync for the project being viewed.
        projectSessions: sessions,
        projectSessionsStatusByProject: {
          ...s.projectSessionsStatusByProject,
          [projectId]: { phase: "ready", generation },
        },
      }));
    } catch (e) {
      if (isStale()) return;
      const message = e instanceof Error ? e.message : String(e);
      set((s) => ({
        projectSessionsStatusByProject: {
          ...s.projectSessionsStatusByProject,
          [projectId]: { phase: "error", generation, error: message },
        },
      }));
    }
  },

  loadTranscript: async (threadId, opts) => {
    if (get().source !== "daemon" || !threadId) return;
    const quiet = opts?.quiet === true || opts?.append === true;
    // Append polls have continuity → safe to demote; short peeks are elevate-only.
    const approvalStatusPolicy: ApprovalStatusPolicy =
      opts?.approvalStatusPolicy ??
      (opts?.append ? "sync" : "elevate-only");
    const prev = get().transcriptStatusByThread[threadId];
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () =>
      get().transcriptStatusByThread[threadId]?.generation !== generation;

    set((s) => ({
      transcriptStatusByThread: {
        ...s.transcriptStatusByThread,
        [threadId]: next,
      },
    }));

    try {
      const existing = get().transcriptsByThread[threadId] ?? [];
      const session =
        get().projectSessions.find((s) => s.id === threadId) ??
        Object.values(get().projectSessionsByProject)
          .flat()
          .find((s) => s.id === threadId) ??
        Object.values(get().sessionsByConversation)
          .flat()
          .find((s) => s.id === threadId);

      // Daemon `from_seq` is exclusive: start = from_seq + 1.
      // Long sessions (10k+ events) must load the *tail*, not the head.
      let fromSeq: number | undefined;
      if (opts?.append && existing.length > 0) {
        fromSeq = Math.max(...existing.map((i) => i.seq));
      } else {
        const lastSeq = session?.messageCount ?? 0;
        const window = opts?.tailWindow ?? 400;
        if (lastSeq > window) {
          fromSeq = lastSeq - window;
        }
      }

      const page = await daemonApi.readTranscript(threadId, fromSeq, 500);
      if (isStale()) return;

      set((s) => {
        const prevItems = opts?.append
          ? (s.transcriptsByThread[threadId] ?? [])
          : [];
        // When replacing (non-append peek), merge with prior items so a short
        // tail window cannot drop an already-known pending approval card.
        const base =
          opts?.append
            ? prevItems
            : approvalStatusPolicy === "elevate-only"
              ? (s.transcriptsByThread[threadId] ?? [])
              : [];
        const merged = [...base, ...page.items];
        const seen = new Set<string>();
        const items = merged.filter((it) => {
          if (seen.has(it.id)) return false;
          seen.add(it.id);
          return true;
        });
        const hasPendingApproval = transcriptHasPendingApproval(items);
        const mapSess = (sess: ProjectSession): ProjectSession => {
          if (sess.id !== threadId) return sess;
          const nextStatus = nextSessionStatusAfterTranscript({
            current: sess.status,
            hasPendingApproval,
            policy: approvalStatusPolicy,
          });
          return nextStatus === sess.status
            ? sess
            : { ...sess, status: nextStatus };
        };
        const sessionsByConversation = { ...s.sessionsByConversation };
        for (const [cid, list] of Object.entries(sessionsByConversation)) {
          sessionsByConversation[cid] = list.map(mapSess);
        }
        const projectSessionsByProject = { ...s.projectSessionsByProject };
        for (const [pid, list] of Object.entries(projectSessionsByProject)) {
          projectSessionsByProject[pid] = list.map(mapSess);
        }
        const projectSessions = (
          s.projectSessionsByProject[
            Object.keys(projectSessionsByProject).find((pid) =>
              projectSessionsByProject[pid]?.some((x) => x.id === threadId),
            ) ?? ""
          ] ?? s.projectSessions
        ).map(mapSess);
        const convId = session?.conversationId ?? "";
        const wasNeedsApproval = session?.status === "needs_approval";
        const demoted =
          wasNeedsApproval &&
          !hasPendingApproval &&
          approvalStatusPolicy === "sync";
        let conversations = s.conversations;
        if (convId && hasPendingApproval) {
          conversations = patchLocalConversation(s.conversations, convId, {
            approvalCount: Math.max(
              s.conversations.find((c) => c.id === convId)?.approvalCount ?? 0,
              1,
            ),
          });
        } else if (convId && demoted) {
          conversations = patchLocalConversation(s.conversations, convId, {
            approvalCount: Math.max(
              (s.conversations.find((c) => c.id === convId)?.approvalCount ??
                1) - 1,
              0,
            ),
          });
        }
        return {
          transcriptsByThread: {
            ...s.transcriptsByThread,
            [threadId]: items,
          },
          transcriptStatusByThread: {
            ...s.transcriptStatusByThread,
            [threadId]: { phase: "ready", generation },
          },
          sessionsByConversation,
          projectSessionsByProject,
          projectSessions,
          conversations,
        };
      });
    } catch (e) {
      if (isStale()) return;
      const message = e instanceof Error ? e.message : String(e);
      set((s) => ({
        transcriptStatusByThread: {
          ...s.transcriptStatusByThread,
          [threadId]: { phase: "error", generation, error: message },
        },
      }));
    }
  },

  loadAttentionSessions: async (opts) => {
    if (get().source !== "daemon") return;
    const quiet = opts?.quiet === true;
    const prev = get().attentionStatus;
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () => get().attentionStatus.generation !== generation;

    set({ attentionStatus: next });
    try {
      const projects = get().projects;
      const chunks = await Promise.all(
        projects.map(async (p) => {
          try {
            return (await daemonApi.listProjectSessions(p.id)).map(toUiSession);
          } catch {
            return [] as ProjectSession[];
          }
        }),
      );
      if (isStale()) return;
      const attentionSessions = chunks
        .flat()
        .filter(
          (s) =>
            !s.parentId &&
            (s.status === "needs_approval" ||
              s.status === "failed" ||
              s.status === "suspended"),
        )
        .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0));
      set({
        attentionSessions,
        attentionStatus: { phase: "ready", generation },
      });
    } catch (e) {
      if (isStale()) return;
      set({
        attentionStatus: {
          phase: "error",
          generation,
          error: e instanceof Error ? e.message : String(e),
        },
      });
    }
  },

  resolveApproval: async (threadId, requestId, decision) => {
    if (get().source !== "daemon") return;
    await daemonApi.resolveApproval(requestId, threadId, { decision });
    // Drop local approval cards for this request; poll will refresh live state.
    set((s) => {
      const items = (s.transcriptsByThread[threadId] ?? []).map((it) =>
        it.requestId === requestId && it.kind === "approval"
          ? {
              ...it,
              kind: "status",
              text: `Approval ${decision}`,
              title: "Approval",
              requestId: null,
            }
          : it,
      );
      const stillPending = transcriptHasPendingApproval(items);
      const session =
        s.projectSessions.find((x) => x.id === threadId) ??
        Object.values(s.projectSessionsByProject)
          .flat()
          .find((x) => x.id === threadId) ??
        Object.values(s.sessionsByConversation)
          .flat()
          .find((x) => x.id === threadId);
      const convId = session?.conversationId;
      const mapSess = (sess: ProjectSession) =>
        sess.id === threadId && !stillPending
          ? { ...sess, status: "running" as SessionStatus }
          : sess;
      const projectSessionsByProject = { ...s.projectSessionsByProject };
      for (const [pid, list] of Object.entries(projectSessionsByProject)) {
        projectSessionsByProject[pid] = list.map(mapSess);
      }
      const sessionsByConversation = { ...s.sessionsByConversation };
      if (convId && sessionsByConversation[convId]) {
        sessionsByConversation[convId] =
          sessionsByConversation[convId]!.map(mapSess);
      } else {
        for (const [cid, list] of Object.entries(sessionsByConversation)) {
          if (list.some((x) => x.id === threadId)) {
            sessionsByConversation[cid] = list.map(mapSess);
          }
        }
      }
      return {
        transcriptsByThread: {
          ...s.transcriptsByThread,
          [threadId]: items,
        },
        projectSessions: s.projectSessions.map(mapSess),
        projectSessionsByProject,
        sessionsByConversation,
        conversations: convId
          ? patchLocalConversation(s.conversations, convId, {
              approvalCount: stillPending
                ? Math.max(
                    (s.conversations.find((c) => c.id === convId)
                      ?.approvalCount ?? 1) - 1,
                    0,
                  )
                : 0,
            })
          : s.conversations,
      };
    });
    // Pull fresh tail after the agent continues.
    await get().loadTranscript(threadId, { tailWindow: 200, quiet: true });
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

  loadClis: async (opts) => {
    if (get().source !== "daemon") return;
    const quiet = opts?.quiet === true;
    const prev = get().clisStatus;
    const { next, generation } = bumpStatus(prev, quiet);
    const isStale = () => get().clisStatus.generation !== generation;
    set({ clisStatus: next });
    try {
      const clis = (await daemonApi.listClis()).map((c) => ({
        agent: c.agent,
        installed: c.installed,
        status: c.status,
      }));
      if (isStale()) return;
      set({ clis, clisStatus: { phase: "ready", generation } });
    } catch (e) {
      if (isStale()) return;
      set({
        clisStatus: {
          phase: "error",
          generation,
          error: e instanceof Error ? e.message : String(e),
        },
      });
    }
  },

  applyIngestEvent: (ev) => {
    if (!ev.threadId) return;
    set((s) => {
      const prevItems = s.transcriptsByThread[ev.threadId] ?? [];
      const items = mergeTranscriptItems(prevItems, ev.items ?? []);
      const hasPending =
        ev.hasPendingApproval || transcriptHasPendingApproval(items);
      const patchStatus = (sess: ProjectSession): ProjectSession => {
        if (sess.id !== ev.threadId) return sess;
        if (hasPending && sess.status !== "needs_approval") {
          return { ...sess, status: "needs_approval" };
        }
        return sess;
      };
      const lists = mapSessionStatusInLists(s, ev.threadId, patchStatus);
      const session =
        Object.values(lists.sessionsByConversation)
          .flat()
          .find((x) => x.id === ev.threadId) ??
        lists.projectSessions.find((x) => x.id === ev.threadId);
      const convId = session?.conversationId ?? "";
      return {
        transcriptsByThread: {
          ...s.transcriptsByThread,
          [ev.threadId]: items,
        },
        ...lists,
        conversations:
          convId && hasPending
            ? patchLocalConversation(s.conversations, convId, {
                approvalCount: Math.max(
                  s.conversations.find((c) => c.id === convId)?.approvalCount ??
                    0,
                  1,
                ),
              })
            : s.conversations,
      };
    });
  },

  applyManagerEvent: (ev) => {
    if (ev.kind === "threadStateChanged") {
      const status = coerceUiSessionStatus(ev.status);
      set((s) => {
        // Don't demote needs_approval → running from manager alone (daemon
        // stays Running during Grok permission/plan). Elevate/clear via ingest.
        const patch = (sess: ProjectSession): ProjectSession => {
          if (sess.id !== ev.threadId) return sess;
          if (
            sess.status === "needs_approval" &&
            (status === "running" || status === "idle")
          ) {
            return sess;
          }
          if (sess.status === status) return sess;
          return { ...sess, status, lastTsMs: ev.atMs || sess.lastTsMs };
        };
        return mapSessionStatusInLists(s, ev.threadId, patch);
      });
      return;
    }
    if (ev.kind === "threadClosed") {
      set((s) =>
        mapSessionStatusInLists(s, ev.threadId, (sess) =>
          sess.id === ev.threadId
            ? { ...sess, status: "done" as SessionStatus }
            : sess,
        ),
      );
      return;
    }
    if (ev.kind === "instanceCrashed") {
      const ids = new Set(ev.affectedThreadIds ?? []);
      set((s) => {
        const mapList = (list: ProjectSession[]) =>
          list.map((sess) =>
            ids.has(sess.id)
              ? { ...sess, status: "suspended" as SessionStatus }
              : sess,
          );
        const sessionsByConversation = { ...s.sessionsByConversation };
        for (const [cid, list] of Object.entries(sessionsByConversation)) {
          sessionsByConversation[cid] = mapList(list);
        }
        const projectSessionsByProject = { ...s.projectSessionsByProject };
        for (const [pid, list] of Object.entries(projectSessionsByProject)) {
          projectSessionsByProject[pid] = mapList(list);
        }
        return {
          sessionsByConversation,
          projectSessionsByProject,
          projectSessions: mapList(s.projectSessions),
        };
      });
      return;
    }
    if (ev.kind === "threadAdded") {
      // Session list hydrate is owned by conversation/project loaders; status
      // will arrive via ThreadStateChanged. No-op if unknown.
      return;
    }
  },

  applyConversationEvent: (ev) => {
    if (!ev.conversationId) return;
    // Debounced quiet re-list of chat_messages for the dirty conversation.
    const id = ev.conversationId;
    const key = `conv-refresh-${id}`;
    const w = window as unknown as {
      __minosConvRefreshTimers?: Record<string, number>;
    };
    w.__minosConvRefreshTimers = w.__minosConvRefreshTimers ?? {};
    const timers = w.__minosConvRefreshTimers;
    if (timers[key]) window.clearTimeout(timers[key]);
    timers[key] = window.setTimeout(() => {
      void get().loadConversationDetail(id, { quiet: true });
    }, 200);
  },

  sendMessage: async (conversationId, body) => {
    const messageBody = body.trimEnd();
    if (!messageBody.trim()) return;

    if (get().source !== "daemon") {
      const msg: TimelineMessage = {
        id: `local-${Date.now()}`,
        role: "user",
        body: messageBody,
        time: "now",
      };
      set((s) => ({
        messagesByConversation: {
          ...s.messagesByConversation,
          [conversationId]: [
            ...(s.messagesByConversation[conversationId] ?? []),
            msg,
          ],
        },
      }));
      return;
    }

    const conv = get().conversations.find((c) => c.id === conversationId);
    const project = get().projects.find((p) => p.id === conv?.projectId);
    if (!conv || !project) {
      throw new Error("conversation or project not found");
    }

    const routed = parseAgentRouting(messageBody);
    let agent: KnownAgent | null = routed?.target.agent ?? null;
    let prompt = routed?.prompt ?? messageBody;

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

    const optimistic: TimelineMessage = {
      id: `opt-${Date.now()}`,
      role: "user",
      body: messageBody,
      time: "now",
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

    try {
      await daemonApi.appendUserMessage(conversationId, messageBody);

      let threadId: string | undefined;
      const sessions = get().sessionsByConversation[conversationId] ?? [];
      if (routed?.target.threadShortId) {
        const match = sessions.find(
          (s) =>
            s.agent === agent &&
            s.status !== "done" &&
            (s.shortId === routed.target.threadShortId ||
              s.id.endsWith(routed.target.threadShortId!) ||
              s.id.startsWith(routed.target.threadShortId!)),
        );
        if (!match) {
          throw new Error(
            `No existing ${agent} session matches #${routed.target.threadShortId}`,
          );
        }
        threadId = match.id;
      } else {
        // Reuse most recent non-closed session for this agent when present
        // (parity with continuing the same thread after TUI/Desktop restart).
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
          threadId = reusable.id;
        } else {
          const started = await daemonApi.startAgentInConversation(
            conversationId,
            agent,
            project.workspacePath,
          );
          threadId = started.threadId;
        }
      }

      if (prompt.trim()) {
        // Reattach only — user text wins over any pending auto-continue flag.
        try {
          await daemonApi.resumeThread(threadId, false);
        } catch {
          /* not needed when already live */
        }
        await daemonApi.sendUserMessage(threadId, prompt);
      }

      await get().loadConversationDetail(conversationId);
      await get().loadConversations(conv.projectId);
      set({ actionError: null });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      await get().loadConversationDetail(conversationId);
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
      return created.id;
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set({ actionError: message });
      throw e;
    }
  },
}));
