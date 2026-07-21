import { invoke } from "@tauri-apps/api/core";

/** True when running inside the Tauri WebView (not plain Vite browser). */
export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

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
  runningCount?: number;
  approvalCount?: number;
};

export type DaemonMention = {
  agent: string;
  threadId?: string | null;
  threadShortId?: string | null;
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
  threadId: string;
  items: TranscriptItem[];
  nextSeq: number | null;
};

/** Push: live ingest frame assembled by the Tauri bridge (TUI-parity). */
export type DaemonIngestEvent = {
  threadId: string;
  seq: number;
  agent: string;
  tsMs: number;
  items: TranscriptItem[];
  hasPendingApproval: boolean;
};

/** Push: thread lifecycle / status. */
export type DaemonManagerEvent =
  | {
      kind: "threadAdded";
      threadId: string;
      agent: string;
      parentThreadId?: string | null;
      workspace: string;
    }
  | {
      kind: "threadStateChanged";
      threadId: string;
      status: string;
      atMs: number;
    }
  | { kind: "threadClosed"; threadId: string }
  | { kind: "instanceCrashed"; affectedThreadIds: string[] };

/** Push: conversation chat_messages dirty. */
export type DaemonConversationEvent = {
  conversationId: string;
  messageSeq: number;
};

export const DAEMON_EVENT = {
  ingest: "daemon://ingest",
  manager: "daemon://manager",
  conversation: "daemon://conversation",
} as const;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error("not running in Tauri");
  }
  return invoke<T>(cmd, args);
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
  listSessions: (conversationId: string) =>
    call<DaemonSession[]>("daemon_list_sessions", { conversationId }),
  createConversation: (projectId: string, title: string) =>
    call<DaemonConversation>("daemon_create_conversation", {
      projectId,
      title,
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
  appendUserMessage: (conversationId: string, body: string) =>
    call<void>("daemon_append_user_message", { conversationId, body }),
  listClis: () =>
    call<
      {
        agent: string;
        installed: boolean;
        path: string | null;
        version: string | null;
        status: string;
      }[]
    >("daemon_list_clis"),
  startAgentInConversation: (
    conversationId: string,
    agent: string,
    workspace: string,
    opts?: { model?: string; reasoningEffort?: string; instructions?: string },
  ) =>
    call<{ threadId: string; cwd: string }>(
      "daemon_start_agent_in_conversation",
      {
        conversationId,
        agent,
        workspace,
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
  sendUserMessage: (threadId: string, text: string) =>
    call<void>("daemon_send_user_message", { threadId, text }),
  resumeThread: (threadId: string, autoContinue = false) =>
    call<void>("daemon_resume_thread", {
      threadId,
      autoContinue,
    }),
  resolveApproval: (
    requestId: string,
    threadId: string,
    decision: Record<string, unknown>,
  ) =>
    call<void>("daemon_resolve_approval", {
      requestId,
      threadId,
      decision,
    }),
  respondOpencodePermission: (
    threadId: string,
    permissionId: string,
    response: string,
  ) =>
    call<void>("daemon_respond_opencode_permission", {
      threadId,
      permissionId,
      response,
    }),
  respondOpencodeQuestion: (
    threadId: string,
    questionId: string,
    answers: string[][],
  ) =>
    call<void>("daemon_respond_opencode_question", {
      threadId,
      questionId,
      answers,
    }),
  listProjectSessions: (projectId: string) =>
    call<DaemonSession[]>("daemon_list_project_sessions", { projectId }),
  readTranscript: (
    threadId: string,
    fromSeq?: number | null,
    limit?: number,
    opts?: { full?: boolean },
  ) =>
    call<TranscriptPage>("daemon_read_transcript", {
      threadId,
      fromSeq: fromSeq ?? null,
      limit: limit ?? 500,
      full: opts?.full === true,
    }),
};
