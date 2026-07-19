import { invoke } from "@tauri-apps/api/core";

/** True when running inside the Tauri WebView (not plain Vite browser). */
export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

export type DaemonConnection = {
  connected: boolean;
  endpoint: string | null;
  error: string | null;
  /** discovery | managed | explicit | error */
  source: string;
  /** This process owns an in-process daemon (TUI-style). */
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

export type DaemonMessage = {
  id: string;
  role: string;
  agent: string | null;
  sessionId: string | null;
  body: string;
  time: string;
  createdAtMs: number;
  kind: string;
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
  /** Pending approval request id when kind === "approval". */
  requestId?: string | null;
  approvalMethod?: string | null;
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
  listMessages: (conversationId: string) =>
    call<DaemonMessage[]>("daemon_list_messages", { conversationId }),
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
  ) =>
    call<{ threadId: string; cwd: string }>(
      "daemon_start_agent_in_conversation",
      { conversationId, agent, workspace },
    ),
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
  listProjectSessions: (projectId: string) =>
    call<DaemonSession[]>("daemon_list_project_sessions", { projectId }),
  readTranscript: (
    threadId: string,
    fromSeq?: number | null,
    limit?: number,
  ) =>
    call<TranscriptPage>("daemon_read_transcript", {
      threadId,
      fromSeq: fromSeq ?? null,
      limit: limit ?? 500,
    }),
};
