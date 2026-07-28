import { invokeDaemon } from "@/shared/api/invoke";

/** Re-export for existing callers; definition lives in `runtime.ts`. */
export { isTauriRuntime } from "./runtime";

/**
 * Raw Tauri bridge connection (implementation detail).
 * Product UI should use `deriveHostPresence` from `host-status.ts`
 * instead of showing `managed` / discovery to end users.
 */
export type DaemonConnection = {
  connected: boolean;
  endpoint: string | null;
  error: string | null;
  /** discovery | managed | explicit | error */
  source: string;
  /** This process owns an in-process daemon (TUI-style). Diagnostics only. */
  managed: boolean;
};

export type DaemonProject = {
  id: string;
  name: string;
  workspacePath: string;
  conversationCount: number;
  runningAgents: number;
  needsAttention: number;
  updatedAtMs: number;
};

export type DaemonConversation = {
  id: string;
  projectId: string;
  title: string;
  preview: string;
  updatedAt: string;
  updatedAtMs: number;
  messageCount: number;
  agentSessionCount: number;
  participatingAgents: string[];
  priority?: string | null;
  progress?: string;
  branch?: string | null;
  worktree?: string | null;
  gitMode?: string | null;
  gitDirty?: boolean | null;
  gitHead?: string | null;
  runningCount?: number;
  approvalCount?: number;
};

export type DaemonMention = {
  agent: string;
  sessionId?: string | null;
  sessionShortId?: string | null;
};

export type DaemonReactionActor = {
  actorId: string;
  actorKind: string;
  displayName: string;
};

export type DaemonReactionGroup = {
  emoji: string;
  count: number;
  reactedByMe: boolean;
  actors?: DaemonReactionActor[];
};

/**
 * Structured git milestone embedded in a conversation message.
 * Canonical shape matches `minos_protocol::GitActivity` (snake_case wire JSON).
 */
export type DaemonGitActivity = {
  kind:
    | "worktree_created"
    | "commits_made"
    | "pr_opened"
    | "checks_failed"
    | "ready_for_review"
    | "merged";
  branch?: string;
  worktree_path?: string;
  base_branch?: string;
  count?: number;
  subjects?: string[];
  head?: string;
  url?: string;
  number?: number;
  title?: string;
  summary?: string;
  merge_commit?: string;
};

export type DaemonMessage = {
  id: string;
  /** Durable timeline sort key; always display ASC by this field. */
  messageSeq: number;
  role: string;
  agent: string | null;
  sessionId: string | null;
  body: string;
  time: string;
  createdAtMs: number;
  kind: string;
  replyToMessageId?: string | null;
  delegationId?: string | null;
  mentions?: DaemonMention[];
  /** Aggregated reactions from local daemon (empty when none). */
  reactions?: DaemonReactionGroup[];
  /** Structured git milestone when kind is git_activity. */
  gitActivity?: DaemonGitActivity | null;
};

export type DaemonGitStatus = {
  path: string;
  branch?: string | null;
  head?: string | null;
  shortHead?: string | null;
  dirty: boolean;
  hasUntracked: boolean;
  aheadCount: number;
  behindCount: number;
  upstream?: string | null;
  isLinkedWorktree: boolean;
  conversation?: DaemonConversation | null;
};

export type DaemonToggleReactionResult = {
  messageId: string;
  conversationId: string;
  reactions: DaemonReactionGroup[];
};

/** One page of conversation messages (tail or older). */
export type DaemonMessagePage = {
  messages: DaemonMessage[];
  /** True when more messages exist before this page. */
  hasMore: boolean;
};

export type DaemonSession = {
  id: string;
  conversationId: string;
  conversationTitle?: string | null;
  agent: string;
  shortId: string;
  status: string;
  model: string;
  parentId: string | null;
  summary: string;
  messageCount: number;
  firstTsMs?: number;
  lastTsMs?: number;
  needsContinue?: boolean;
};

export type TranscriptOption = {
  label: string;
  description?: string | null;
};

export type TranscriptItem = {
  id: string;
  kind: string;
  role: string | null;
  text: string;
  detail?: string | null;
  title?: string | null;
  tsMs: number;
  seq: number;
  messageId?: string | null;
  /** Pending approval / question request id. */
  requestId?: string | null;
  /** e.g. x.ai/exit_plan_mode, session/request_permission, opencode/question */
  approvalMethod?: string | null;
  /** Structured options for question cards. */
  options?: TranscriptOption[] | null;
  /** OpenCode permission accept token. */
  approveResponse?: string | null;
  /** OpenCode permission decline token. */
  declineResponse?: string | null;
};

export type TranscriptPage = {
  sessionId: string;
  items: TranscriptItem[];
  nextSeq: number | null;
};

/** Push: live ingest frame assembled by the Tauri bridge (TUI-parity). */
export type DaemonIngestEvent = {
  sessionId: string;
  seq: number;
  agent: string;
  tsMs: number;
  items: TranscriptItem[];
  hasPendingApproval: boolean;
};

/** Push: session lifecycle / status. */
export type DaemonManagerEvent =
  | {
      kind: "sessionAdded";
      sessionId: string;
      agent: string;
      parentSessionId?: string | null;
      workspace: string;
    }
  | {
      kind: "sessionStateChanged";
      sessionId: string;
      status: string;
      atMs: number;
    }
  | { kind: "sessionClosed"; sessionId: string }
  | { kind: "instanceCrashed"; affectedSessionIds: string[] };

/** Push: conversation timeline events (message append or reaction toggle). */
export type DaemonConversationEvent =
  | {
      kind: "messageAppended";
      conversationId: string;
      messageSeq: number;
    }
  | {
      kind: "reactionToggled";
      conversationId: string;
      messageId: string;
      reactions: DaemonReactionGroup[];
    };

/** Push: subscription pump health (livePush gate). */
export type DaemonPushStatusEvent = {
  live: boolean;
};

export const DAEMON_EVENT = {
  ingest: "daemon://ingest",
  manager: "daemon://manager",
  conversation: "daemon://conversation",
  pushStatus: "daemon://push-status",
} as const;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invokeDaemon<T>(cmd, args);
}

export const daemonApi = {
  connect: (url?: string) =>
    call<DaemonConnection>("daemon_connect", { url: url ?? null }),
  status: () => call<DaemonConnection>("daemon_status"),
  listProjects: () => call<DaemonProject[]>("daemon_list_projects"),
  createProject: (workspacePath: string) =>
    call<DaemonProject>("daemon_create_project", { workspacePath }),
  listConversations: (projectId: string) =>
    call<DaemonConversation[]>("daemon_list_conversations", { projectId }),
  /**
   * Page conversation messages (ASC).
   * - No `beforeSeq`: newest `limit` messages (tail).
   * - With `beforeSeq`: older page strictly before that durable seq.
   */
  listMessages: (
    conversationId: string,
    opts?: { beforeSeq?: number; limit?: number },
  ) =>
    call<DaemonMessagePage>("daemon_list_messages", {
      conversationId,
      beforeSeq: opts?.beforeSeq ?? null,
      limit: opts?.limit ?? null,
    }),
  toggleMessageReaction: (messageId: string, emoji: string) =>
    call<DaemonToggleReactionResult>("daemon_toggle_message_reaction", {
      messageId,
      emoji,
    }),
  listSessions: (conversationId: string) =>
    call<DaemonSession[]>("daemon_list_sessions", { conversationId }),
  createConversation: (
    projectId: string,
    title: string,
    opts?: {
      priority?: string | null;
      agents?: string[];
      /** inherit | worktree; omit for daemon default (worktree when repo). */
      gitMode?: string | null;
    },
  ) =>
    call<DaemonConversation>("daemon_create_conversation", {
      projectId,
      title,
      priority: opts?.priority ?? null,
      agents: opts?.agents ?? [],
      gitMode: opts?.gitMode ?? null,
    }),
  gitGetStatus: (
    conversationId: string,
    opts?: { refreshConversation?: boolean },
  ) =>
    call<DaemonGitStatus>("daemon_git_get_status", {
      conversationId,
      refreshConversation: opts?.refreshConversation ?? true,
    }),
  updateConversation: (
    conversationId: string,
    patch: {
      title?: string | null;
      priority?: string | null;
      progress?: string | null;
    },
  ) =>
    call<DaemonConversation>("daemon_update_conversation", {
      conversationId,
      title: patch.title ?? null,
      priority: patch.priority ?? null,
      progress: patch.progress ?? null,
    }),
  appendUserMessage: (
    conversationId: string,
    body: string,
    messageId: string,
  ) =>
    call<{ messageSeq: number }>("daemon_append_user_message", {
      conversationId,
      body,
      messageId,
    }),
  listClis: () =>
    call<
      {
        agent: string;
        displayName: string;
        installed: boolean;
        path: string | null;
        version: string | null;
        status: string;
        supportsModelSelection: boolean;
        supportsReasoningEffort: boolean;
      }[]
    >("daemon_list_clis"),
  startAgentInConversation: (
    conversationId: string,
    agent: string,
    workspace: string,
    opts?: {
      profileId?: string;
      model?: string;
      reasoningEffort?: string;
      instructions?: string;
    },
  ) =>
    call<{ sessionId: string; cwd: string }>(
      "daemon_start_agent_in_conversation",
      {
        conversationId,
        agent,
        workspace,
        profileId: opts?.profileId ?? null,
        model: opts?.model ?? null,
        reasoningEffort: opts?.reasoningEffort ?? null,
        instructions: opts?.instructions ?? null,
      },
    ),
  listModels: (runtime: string) =>
    call<{
      runtime: string;
      models: {
        id: string;
        display_name: string;
        description?: string | null;
        is_default: boolean;
        supported_reasoning_efforts: string[];
        default_reasoning_effort?: string | null;
      }[];
      source: string;
    }>("daemon_list_models", { runtime }),
  listAgentProfiles: () =>
    call<{
      profiles: {
        id: string;
        name: string;
        description: string;
        runtime_agent: string;
        model: string;
        reasoning_effort: string;
        instructions: string;
        created_at_ms: number;
        updated_at_ms: number;
      }[];
    }>("daemon_list_agent_profiles"),
  createAgentProfile: (input: {
    name: string;
    description: string;
    runtimeAgent: string;
    model: string;
    reasoningEffort: string;
    instructions: string;
  }) =>
    call<{
      id: string;
      name: string;
      description: string;
      runtime_agent: string;
      model: string;
      reasoning_effort: string;
      instructions: string;
      created_at_ms: number;
      updated_at_ms: number;
    }>("daemon_create_agent_profile", {
      name: input.name,
      description: input.description,
      runtime_agent: input.runtimeAgent,
      model: input.model,
      reasoning_effort: input.reasoningEffort,
      instructions: input.instructions,
    }),
  deleteAgentProfile: (id: string) =>
    call<void>("daemon_delete_agent_profile", { id }),
  sendUserMessage: (sessionId: string, text: string) =>
    call<void>("daemon_send_user_message", { sessionId, text }),
  resumeSession: (sessionId: string, autoContinue = false) =>
    call<void>("daemon_resume_session", {
      sessionId,
      autoContinue,
    }),
  resolveApproval: (
    requestId: string,
    sessionId: string,
    decision: Record<string, unknown>,
  ) =>
    call<void>("daemon_resolve_approval", {
      requestId,
      sessionId,
      decision,
    }),
  respondOpencodePermission: (
    sessionId: string,
    permissionId: string,
    response: string,
  ) =>
    call<void>("daemon_respond_opencode_permission", {
      sessionId,
      permissionId,
      response,
    }),
  respondOpencodeQuestion: (
    sessionId: string,
    questionId: string,
    answers: string[][],
  ) =>
    call<void>("daemon_respond_opencode_question", {
      sessionId,
      questionId,
      answers,
    }),
  listProjectSessions: (projectId: string) =>
    call<DaemonSession[]>("daemon_list_project_sessions", { projectId }),
  readTranscript: (
    sessionId: string,
    fromSeq?: number | null,
    limit?: number,
    opts?: { full?: boolean },
  ) =>
    call<TranscriptPage>("daemon_read_transcript", {
      sessionId,
      fromSeq: fromSeq ?? null,
      limit: limit ?? 500,
      full: opts?.full === true,
    }),
};
