/** Mock fixtures for the Minos desktop shell (TUI-parity IA). */

export type AgentRuntime =
  | "codex"
  | "claude"
  | "gemini"
  | "opencode"
  | "grok";

export type SessionStatus =
  | "idle"
  | "running"
  | "needs_approval"
  | "suspended"
  | "failed"
  | "done";

export type ConversationBoardColumn =
  | "backlog"
  | "running"
  | "needs_you"
  | "done";

export type ConversationPriority = "high" | "medium" | "low";

export type ConversationProgress =
  | "todo"
  | "in_progress"
  | "in_review"
  | "done";

export type AvatarTone =
  | "green"
  | "blue"
  | "pink"
  | "orange"
  | "purple"
  | "slate"
  | "amber";

export type Project = {
  id: string;
  name: string;
  workspacePath: string;
  conversationCount: number;
  runningAgents: number;
  needsAttention: number;
  /** Last activity timestamp (ms) for WeChat-style list sort. */
  updatedAtMs: number;
  /** True when any conversation has unread or pending approval. */
  hasUnread: boolean;
  /** Max updatedAtMs among conversations that currently need attention. */
  lastAttentionMs: number;
  /**
   * Which host owns this project (plane C).
   * Omit for this Mac (default). Multi-device rows set a device display name.
   */
  hostName?: string;
};

export type Conversation = {
  id: string;
  projectId: string;
  title: string;
  preview: string;
  updatedAt: string;
  /** Raw last-update timestamp (ms) for sorting; `updatedAt` is display-only. */
  updatedAtMs: number;
  unread?: number;
  /** Aggregated message count in the conversation timeline. */
  messageCount: number;
  boardColumn: ConversationBoardColumn;
  agentSessionCount: number;
  /**
   * Runtime agents on the conversation roster (membership SSOT).
   * @mention / start_agent are gated on this list — not on installed CLIs alone.
   */
  participatingAgents: string[];
  runningCount: number;
  approvalCount: number;
  /** Git branch for this conversation work unit. */
  branch?: string;
  /** Optional linked git worktree path. */
  worktree?: string;
  /** Git isolation mode: inherit | worktree. */
  gitMode?: string;
  /** Cached dirty flag from last git status refresh. */
  gitDirty?: boolean;
  /** Cached HEAD from last git status refresh. */
  gitHead?: string;
  /** User priority tag; omit when unset. */
  priority?: ConversationPriority;
  /** Workflow progress (default todo). */
  progress?: ConversationProgress;
};

export type TimelineMention = {
  agent: string;
  sessionId?: string;
  sessionShortId?: string;
};

export type DeliveryStatus = "sending" | "sent" | "failed";

/** Git milestone shown as a dedicated timeline card. */
export type TimelineGitActivity = {
  kind:
    | "worktree_created"
    | "commits_made"
    | "pr_opened"
    | "checks_failed"
    | "ready_for_review"
    | "merged";
  branch?: string;
  worktreePath?: string;
  baseBranch?: string;
  count?: number;
  subjects?: string[];
  head?: string;
  url?: string;
  number?: number;
  title?: string;
  summary?: string;
  mergeCommit?: string;
};

/** Viewer-resolved reaction aggregate carried from Hub cold pull / live frames. */
export type TimelineReactionActor = {
  id: string;
  displayName: string;
};

export type TimelineReactionGroup = {
  emoji: string;
  count: number;
  reactedByMe: boolean;
  actors: TimelineReactionActor[];
};

export type TimelineMessage = {
  id: string;
  /** Durable sort key from daemon; optimistic rows may omit until reload. */
  messageSeq?: number;
  role: "user" | "agent" | "system";
  agent?: AgentRuntime;
  sessionId?: string;
  body: string;
  time: string;
  createdAtMs?: number;
  kind?: "text" | "tool_summary" | "approval" | "git_activity" | "system";
  replyToMessageId?: string;
  delegationId?: string;
  mentions?: TimelineMention[];
  gitActivity?: TimelineGitActivity;
  /**
   * Hub (or daemon) reaction aggregates for cold hydrate into reaction-store.
   * Optional — live frames may omit until a reaction event arrives.
   */
  reactions?: TimelineReactionGroup[];
  /**
   * Local delivery lifecycle for user messages.
   * - `sending`: optimistic bubble shown immediately, awaiting append/RPC ack.
   * - `failed`: append or send pipeline threw; row stays until retry succeeds.
   * - `sent`: append succeeded (durable). Omitted on durable rows loaded from
   *   daemon — absence is treated as `sent` by render.
   */
  deliveryStatus?: DeliveryStatus;
};

export type AgentSession = {
  id: string;
  conversationId: string;
  agent: AgentRuntime;
  shortId: string;
  status: SessionStatus;
  model: string;
  parentId?: string;
  summary: string;
  lastTool?: string;
  /** Host should auto-continue after process-death recovery. */
  needsContinue?: boolean;
};

export type HostStatus = {
  relay: "connected" | "disconnected";
  daemonVersion: string;
  pairedDevices: number;
};

export type InstalledRuntime = {
  agent: AgentRuntime;
  installed: boolean;
  path?: string;
  version?: string;
};

export const hostStatus: HostStatus = {
  relay: "connected",
  daemonVersion: "0.1.0",
  pairedDevices: 1,
};

export const installedRuntimes: InstalledRuntime[] = [
  { agent: "codex", installed: true, path: "/opt/homebrew/bin/codex", version: "0.44.2" },
  { agent: "claude", installed: true, path: "/opt/homebrew/bin/claude", version: "1.0.88" },
  { agent: "gemini", installed: true, path: "~/.local/bin/gemini", version: "0.9.1" },
  { agent: "opencode", installed: false },
  { agent: "grok", installed: true, path: "/opt/homebrew/bin/grok", version: "0.2.0" },
];

const NOW_MS = Date.now();

export const projects: Project[] = [
  {
    id: "proj-minos",
    name: "minos",
    workspacePath: "~/develop/github.com/minos",
    conversationCount: 4,
    runningAgents: 2,
    needsAttention: 2,
    updatedAtMs: NOW_MS - 2 * 60_000,
    hasUnread: true,
    lastAttentionMs: NOW_MS - 2 * 60_000,
  },
  {
    id: "proj-landing",
    name: "marketing-site",
    workspacePath: "~/develop/github.com/marketing-site",
    conversationCount: 2,
    runningAgents: 0,
    needsAttention: 0,
    updatedAtMs: NOW_MS - 3 * 60 * 60_000,
    hasUnread: false,
    lastAttentionMs: 0,
  },
  {
    id: "proj-sdk",
    name: "client-sdk",
    workspacePath: "~/develop/work/client-sdk",
    conversationCount: 1,
    runningAgents: 1,
    needsAttention: 1,
    updatedAtMs: NOW_MS - 12 * 60_000,
    hasUnread: true,
    lastAttentionMs: NOW_MS - 12 * 60_000,
  },
];

export const conversations: Conversation[] = [
  {
    id: "conv-auth",
    projectId: "proj-minos",
    title: "JWT auth refactor",
    preview: "@codex finished route handlers; @claude reviewing tests…",
    updatedAt: "2m",
    updatedAtMs: NOW_MS - 2 * 60_000,
    unread: 2,
    messageCount: 14,
    boardColumn: "running",
    agentSessionCount: 2,
    participatingAgents: ["codex", "claude"],
    runningCount: 1,
    approvalCount: 1,
    branch: "feature/jwt-auth",
    worktree: "~/wt/minos-jwt-auth",
    priority: "high",
    progress: "in_progress",
  },
  {
    id: "conv-desktop",
    projectId: "proj-minos",
    title: "Desktop shell IA",
    preview: "You: map TUI nav to multi-pane desktop…",
    updatedAt: "18m",
    updatedAtMs: NOW_MS - 18 * 60_000,
    messageCount: 6,
    boardColumn: "needs_you",
    agentSessionCount: 1,
    participatingAgents: ["codex"],
    runningCount: 0,
    approvalCount: 1,
    branch: "feature/mobile-auth-and-agent-session",
    priority: "high",
    progress: "in_review",
  },
  {
    id: "conv-ingest",
    projectId: "proj-minos",
    title: "Ingest sync rework",
    preview: "codex: checkpoint coalescing landed",
    updatedAt: "1h",
    updatedAtMs: NOW_MS - 60 * 60_000,
    messageCount: 9,
    boardColumn: "done",
    agentSessionCount: 1,
    participatingAgents: ["codex"],
    runningCount: 0,
    approvalCount: 0,
    branch: "feat/ingest-sync",
    priority: "medium",
    progress: "done",
  },
  {
    id: "conv-docs",
    projectId: "proj-minos",
    title: "Architecture docs pass",
    preview: "No agents yet — draft outline ready",
    updatedAt: "Yesterday",
    updatedAtMs: NOW_MS - 24 * 60 * 60_000,
    messageCount: 1,
    boardColumn: "backlog",
    agentSessionCount: 0,
    participatingAgents: [],
    runningCount: 0,
    approvalCount: 0,
    branch: "docs/architecture",
    priority: "low",
    progress: "todo",
  },
  {
    id: "conv-hero",
    projectId: "proj-landing",
    title: "Hero section rewrite",
    preview: "Waiting to start an agent",
    updatedAt: "3h",
    updatedAtMs: NOW_MS - 3 * 60 * 60_000,
    messageCount: 1,
    boardColumn: "backlog",
    agentSessionCount: 0,
    participatingAgents: ["claude"],
    runningCount: 0,
    approvalCount: 0,
    branch: "main",
    priority: "medium",
    progress: "todo",
  },
  {
    id: "conv-seo",
    projectId: "proj-landing",
    title: "SEO meta tags",
    preview: "gemini: drafted Open Graph tags",
    updatedAt: "5h",
    updatedAtMs: NOW_MS - 5 * 60 * 60_000,
    messageCount: 4,
    boardColumn: "done",
    agentSessionCount: 1,
    participatingAgents: ["gemini"],
    runningCount: 0,
    approvalCount: 0,
    branch: "chore/seo-meta",
    priority: "low",
    progress: "done",
  },
  {
    id: "conv-sdk",
    projectId: "proj-sdk",
    title: "Retry policy for WS",
    preview: "@grok needs approval to edit reconnect.rs",
    updatedAt: "12m",
    updatedAtMs: NOW_MS - 12 * 60_000,
    unread: 1,
    messageCount: 3,
    boardColumn: "needs_you",
    agentSessionCount: 1,
    participatingAgents: ["grok"],
    runningCount: 1,
    approvalCount: 1,
    branch: "fix/ws-backoff",
    worktree: "~/wt/sdk-ws-retry",
    priority: "high",
    progress: "in_progress",
  },
];

export const agentSessions: AgentSession[] = [
  {
    id: "sess-codex-1",
    conversationId: "conv-auth",
    agent: "codex",
    shortId: "a1b2",
    status: "needs_approval",
    model: "GPT-5.5",
    summary: "Refactoring auth middleware + route handlers",
    lastTool: "apply_patch src/auth/jwt.rs",
  },
  {
    id: "sess-claude-1",
    conversationId: "conv-auth",
    agent: "claude",
    shortId: "c9f0",
    status: "running",
    model: "Opus",
    summary: "Writing integration tests for token refresh",
    lastTool: "Bash cargo test -p minos-backend auth",
  },
  {
    id: "sess-claude-sub",
    conversationId: "conv-auth",
    agent: "claude",
    shortId: "c9f0-s1",
    status: "running",
    model: "Opus",
    parentId: "sess-claude-1",
    summary: "Subagent: fixture helpers",
    lastTool: "Edit tests/auth_helper.rs",
  },
  {
    id: "sess-codex-2",
    conversationId: "conv-desktop",
    agent: "codex",
    shortId: "d4e5",
    status: "needs_approval",
    model: "GPT-5.5",
    summary: "Scaffold apps/desktop shell components",
    lastTool: "Write apps/desktop/src/App.tsx",
  },
  {
    id: "sess-codex-3",
    conversationId: "conv-ingest",
    agent: "codex",
    shortId: "i7j8",
    status: "done",
    model: "GPT-5.5",
    summary: "Ingest coalescer + checkpoint",
  },
  {
    id: "sess-grok-1",
    conversationId: "conv-sdk",
    agent: "grok",
    shortId: "g2h3",
    status: "needs_approval",
    model: "Grok",
    summary: "WS retry backoff",
    lastTool: "apply_patch reconnect.rs",
  },
];

/** Stable mock wall-clock anchors so grouping + day dividers work in browser mock. */
const MOCK_TODAY = (() => {
  const d = new Date();
  d.setHours(10, 0, 0, 0);
  return d.getTime();
})();
const MOCK_YESTERDAY = MOCK_TODAY - 24 * 60 * 60 * 1000;

export const timelineByConversation: Record<string, TimelineMessage[]> = {
  "conv-auth": [
    {
      id: "m1",
      role: "user",
      body: "@codex Refactor session auth to JWT with refresh rotation. Keep the existing /v1/auth routes stable.",
      time: "10:12",
      createdAtMs: MOCK_TODAY + 12 * 60_000,
    },
    {
      id: "m2",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-1",
      body: "On it. I'll map the current cookie flow, introduce access + refresh tokens, and keep route contracts intact.",
      time: "10:12",
      createdAtMs: MOCK_TODAY + 12 * 60_000 + 5_000,
    },
    {
      id: "m3",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-1",
      kind: "tool_summary",
      body: "Read auth/use_case.rs · store/refresh_tokens.rs · http/v1/auth.rs",
      time: "10:13",
      createdAtMs: MOCK_TODAY + 13 * 60_000,
    },
    {
      id: "m4",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-1",
      kind: "approval",
      body: "Permission: apply_patch → crates/minos-backend/src/auth/jwt.rs (create) and refresh_tokens.rs (edit)",
      time: "10:15",
      createdAtMs: MOCK_TODAY + 15 * 60_000,
    },
    {
      id: "m5",
      role: "user",
      body: "@claude After codex lands the handlers, add integration tests for refresh rotation and 401 retry.",
      time: "10:16",
      createdAtMs: MOCK_TODAY + 16 * 60_000,
    },
    {
      id: "m6",
      role: "agent",
      agent: "claude",
      sessionId: "sess-claude-1",
      body: "Starting test plan. I'll cover login → refresh → reuse of rotated token → revoke path.",
      time: "10:16",
      createdAtMs: MOCK_TODAY + 16 * 60_000 + 8_000,
    },
    {
      id: "m7",
      role: "agent",
      agent: "claude",
      sessionId: "sess-claude-1",
      kind: "tool_summary",
      body: "Running · cargo test -p minos-backend auth_endpoints",
      time: "10:18",
      createdAtMs: MOCK_TODAY + 18 * 60_000,
    },
  ],
  "conv-desktop": [
    {
      id: "d1",
      role: "user",
      body: "@codex Build a Tauri desktop shell that mirrors TUI: Project → Conversation → agent sessions.",
      time: "09:40",
      createdAtMs: MOCK_TODAY - 20 * 60_000,
    },
    {
      id: "d2",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-2",
      body: "Scaffolding apps/desktop with multi-pane layout. Mock data first, daemon RPC next.",
      time: "09:41",
      createdAtMs: MOCK_TODAY - 19 * 60_000,
    },
    {
      id: "d3",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-2",
      kind: "approval",
      body: "Permission: write apps/desktop/src/components/** and replace inbox mock IA",
      time: "09:55",
      createdAtMs: MOCK_TODAY - 5 * 60_000,
    },
  ],
  "conv-ingest": [
    {
      id: "i1",
      role: "user",
      body: "@codex Rework ingest sync: live batch + gap manifest, no legacy reconciliator.",
      time: "Yesterday",
      createdAtMs: MOCK_YESTERDAY + 14 * 60 * 60_000,
    },
    {
      id: "i2",
      role: "agent",
      agent: "codex",
      sessionId: "sess-codex-3",
      body: "Done. Coalescer batches deltas; reconnect sends HostGapManifest before pull range.",
      time: "Yesterday",
      createdAtMs: MOCK_YESTERDAY + 14 * 60 * 60_000 + 60_000,
    },
  ],
  "conv-docs": [
    {
      id: "doc1",
      role: "system",
      body: "Conversation created. Mention an installed agent with @ to start a run in this project workspace.",
      time: "Yesterday",
      createdAtMs: MOCK_YESTERDAY + 9 * 60 * 60_000,
    },
  ],
  "conv-hero": [
    {
      id: "h1",
      role: "system",
      body: "Empty conversation. Try @gemini rewrite the hero copy for B2B.",
      time: "3h",
    },
  ],
  "conv-seo": [
    {
      id: "s1",
      role: "user",
      body: "@gemini Add Open Graph and Twitter meta tags to the landing layout.",
      time: "5h",
    },
    {
      id: "s2",
      role: "agent",
      agent: "gemini",
      body: "Drafted meta tags and a small helper for per-page overrides.",
      time: "5h",
    },
  ],
  "conv-sdk": [
    {
      id: "k1",
      role: "user",
      body: "@grok Implement exponential backoff for websocket reconnect in the client SDK.",
      time: "11:02",
    },
    {
      id: "k2",
      role: "agent",
      agent: "grok",
      sessionId: "sess-grok-1",
      kind: "approval",
      body: "Permission: apply_patch → src/reconnect.rs",
      time: "11:08",
    },
  ],
};

export const boardColumns: {
  id: ConversationBoardColumn;
  label: string;
  /** Header chip background only — column body stays neutral. */
  headerBg: string;
  headerText: string;
}[] = [
  {
    id: "backlog",
    label: "Backlog",
    headerBg: "bg-status-suspended/15",
    headerText: "text-status-suspended",
  },
  {
    id: "running",
    label: "Running",
    headerBg: "bg-status-running/20",
    headerText: "text-status-running",
  },
  {
    id: "needs_you",
    label: "Needs you",
    headerBg: "bg-status-approval/15",
    headerText: "text-status-approval",
  },
  {
    id: "done",
    label: "Done",
    headerBg: "bg-status-done/15",
    headerText: "text-status-done",
  },
];

/** Soft brand chips — opacity on solid hues so light/dark themes both work. */
export const agentMeta: Record<
  AgentRuntime,
  { label: string; tone: AvatarTone; color: string }
> = {
  codex: {
    label: "Codex",
    tone: "orange",
    color: "bg-orange-500/15 text-orange-800 dark:text-orange-200",
  },
  claude: {
    label: "Claude",
    tone: "purple",
    color: "bg-violet-500/15 text-violet-800 dark:text-violet-200",
  },
  gemini: {
    label: "Gemini",
    tone: "blue",
    color: "bg-sky-500/15 text-sky-800 dark:text-sky-200",
  },
  opencode: {
    label: "OpenCode",
    tone: "slate",
    color: "bg-ink/10 text-ink-secondary",
  },
  grok: {
    label: "Grok",
    tone: "amber",
    color: "bg-status-running/20 text-status-running",
  },
};

export const toneClasses: Record<AvatarTone, string> = {
  green: "bg-emerald-500/15 text-emerald-800 dark:text-emerald-200",
  blue: "bg-sky-500/15 text-sky-800 dark:text-sky-200",
  pink: "bg-pink-500/15 text-pink-800 dark:text-pink-200",
  orange: "bg-orange-500/15 text-orange-800 dark:text-orange-200",
  purple: "bg-violet-500/15 text-violet-800 dark:text-violet-200",
  slate: "bg-ink/10 text-ink-secondary",
  amber: "bg-status-running/20 text-status-running",
};

export const statusMeta: Record<
  SessionStatus,
  { label: string; dot: string; pill: string }
> = {
  idle: {
    label: "Idle",
    dot: "bg-ink-muted",
    pill: "bg-ink/10 text-ink-secondary",
  },
  running: {
    label: "Running",
    dot: "bg-status-running",
    pill: "bg-status-running/20 text-status-running",
  },
  needs_approval: {
    label: "Needs approval",
    dot: "bg-status-approval",
    pill: "bg-status-approval/15 text-status-approval",
  },
  suspended: {
    label: "Paused",
    dot: "bg-status-suspended",
    pill: "bg-status-suspended/15 text-status-suspended",
  },
  failed: {
    label: "Failed",
    dot: "bg-status-failed",
    pill: "bg-status-failed/15 text-status-failed",
  },
  done: {
    label: "Done",
    dot: "bg-status-done",
    pill: "bg-status-done/20 text-status-done",
  },
};

export function initials(name: string): string {
  return name
    .split(/[\s/_-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
}

export function conversationsForProject(projectId: string): Conversation[] {
  return conversations.filter((c) => c.projectId === projectId);
}

export function sessionsForConversation(conversationId: string): AgentSession[] {
  return agentSessions.filter((s) => s.conversationId === conversationId);
}

export function projectById(projectId: string): Project | undefined {
  return projects.find((p) => p.id === projectId);
}

export function conversationById(id: string): Conversation | undefined {
  return conversations.find((c) => c.id === id);
}

export function totalAttention(): number {
  return projects.reduce((sum, p) => sum + p.needsAttention, 0);
}
