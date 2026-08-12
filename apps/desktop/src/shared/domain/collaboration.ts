/** Desktop collaboration domain types (not fixtures). */

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
  /**
   * Max `updatedAtMs` among conversations that currently need attention.
   * Sort uses `hasUnread` + project `updatedAtMs` only — this is diagnostic /
   * aggregate metadata, not a separate sort key.
   */
  lastAttentionMs: number;
  /**
   * Which host owns this project (plane C).
   * Omit for this Mac (default). Multi-device rows set a device display name.
   */
  hostName?: string;
};

/** Conversation bot member card (membership key = botId). */
export type ParticipatingBot = {
  botId: string;
  name: string;
  runtime: string;
};

/** Derive unique runtime labels from structured bots (host-runtime ensure / badges). */
export function runtimesOfBots(bots: readonly ParticipatingBot[] | undefined): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const b of bots ?? []) {
    const r = b.runtime.trim().toLowerCase();
    if (!r || seen.has(r)) continue;
    seen.add(r);
    out.push(r);
  }
  return out;
}

/** Membership tokens for @ resolve: botId ∪ name ∪ runtime (all lowercased). */
export function membershipTokensOfBots(
  bots: readonly ParticipatingBot[] | undefined,
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const b of bots ?? []) {
    for (const raw of [b.botId, b.name, b.runtime]) {
      const t = raw.trim().toLowerCase();
      if (!t || seen.has(t)) continue;
      seen.add(t);
      out.push(t);
    }
  }
  return out;
}

export type Conversation = {
  id: string;
  projectId: string;
  title: string;
  preview: string;
  /**
   * Last activity epoch ms (sort + list clock SSOT).
   * Format at render with `formatListActivityTime` — never store display strings.
   */
  updatedAtMs: number;
  unread?: number;
  /** Aggregated message count in the conversation timeline. */
  messageCount: number;
  boardColumn: ConversationBoardColumn;
  agentSessionCount: number;
  /**
   * Bot participants on the conversation roster (membership SSOT).
   * Membership key is `botId`; `runtime` is launch/badge only.
   */
  participatingBots: ParticipatingBot[];
  /**
   * @deprecated Derived runtime labels from `participatingBots` for host-runtime
   * ensure / badges. Do not use as membership SSOT.
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
  /** Runtime agent name when kind is agent (legacy workbench rows). */
  agent?: string;
  sessionId?: string;
  sessionShortId?: string;
  /** Structured mention kind from Hub SSOT. */
  kind?: "account" | "agent";
  /** account_id or agent_id depending on kind. */
  targetId?: string;
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
  /**
   * Social total-order key: Hub `message_seq` when linked, host daemon seq only
   * for local-only (unlinked) workbench chat. Host tool/git cards must not use
   * this field for cross-source order — use `anchorCloudMessageSeq` + `suborder`.
   */
  messageSeq?: number;
  /**
   * Host-only cards: place after the Hub bubble with this social seq.
   * Undefined = unanchored (sort after social rows, before optimistic tail).
   */
  anchorCloudMessageSeq?: number;
  /** Among host cards with the same anchor (or unanchored), local order. */
  suborder?: number;
  /** Daemon host seq for host-only pagination (never mixed into Hub before_seq). */
  hostMessageSeq?: number;
  role: "user" | "agent" | "system";
  /** Runtime family (codex/claude/…) for badges / tone; not bot identity. */
  agent?: AgentRuntime;
  /**
   * Hub bot display name (MessageSender::Bot) or human display name when known.
   * Prefer this over runtime-keyed `agentMeta` for author labels.
   */
  senderDisplayName?: string;
  /** Global bot id when role=agent (MessageSender::Bot.bot_id). */
  botId?: string;
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

/** Session list row: AgentSession plus host list metadata. */
export type ProjectSession = AgentSession & {
  conversationTitle?: string;
  firstTsMs?: number;
  lastTsMs?: number;
  /** Session last_seq — used to seek transcript tail. */
  messageCount?: number;
};

