/**
 * Workspace store types (L0–L6 surface).
 * Implementation modules import from here; app still uses @/store/workspace-store.
 */
import type { StoreApi } from "zustand";
import type {
  Conversation,
  ConversationPriority,
  ConversationProgress,
  DeliveryStatus,
  Project,
  TimelineMessage,
  AgentSession,
  ConversationBoardColumn,
} from "@/shared/lib/mock-data";
import type {
  DaemonConnection,
  DaemonConversationEvent,
  DaemonIngestEvent,
  DaemonManagerEvent,
  TranscriptItem,
} from "@/shared/lib/daemon";
import type { SessionEntity } from "@/shared/lib/session-entity";
import type { ApprovalStatusPolicy } from "@/shared/lib/session-status";
import type { MessageHistoryMeta } from "@/shared/lib/message-history";
import type { TranscriptHistoryMeta } from "@/shared/lib/transcript-history";

export type { SessionEntity };

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

export type ProjectSession = AgentSession & {
  conversationTitle?: string;
  firstTsMs?: number;
  lastTsMs?: number;
  /** Session last_seq — used to seek transcript tail. */
  messageCount?: number;
};

export type SessionListSlice = {
  sessionsById: Record<string, SessionEntity>;
  sessionsByConversation: Record<string, ProjectSession[]>;
  projectSessionsByProject: Record<string, ProjectSession[]>;
  attentionSessions: ProjectSession[];
  conversations: Conversation[];
  projects?: Project[];
  attentionStatus?: ResourceFetchStatus;
};

export type WorkspaceState = {
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
  /** Infinite-scroll cursor / loading flags per conversation timeline. */
  messageHistoryByConversation: Record<string, MessageHistoryMeta>;
  sessionsByConversation: Record<string, ProjectSession[]>;
  /** Per-conversation Timeline (messages) fetch status. */
  timelineStatusByConversation: Record<string, ResourceFetchStatus>;
  /** Per-conversation Inspector (sessions) fetch status. */
  inspectorStatusByConversation: Record<string, ResourceFetchStatus>;
  /**
   * Conversations marked dirty by LiveIngress when no Timeline working set
   * exists (no RPC until ensureLoaded).
   */
  timelineDirtyByConversation: Record<string, boolean>;
  /** Project-scoped aggregate sessions (Sessions tab). Keyed by projectId. */
  projectSessionsByProject: Record<string, ProjectSession[]>;
  projectSessionsStatusByProject: Record<string, ResourceFetchStatus>;
  /**
   * L4 SessionEntity map — sole status / hasPendingApproval truth.
   * List caches project from this; never demote approval by scanning missing
   * transcript windows (use entity.hasPendingApproval).
   */
  sessionsById: Record<string, SessionEntity>;
  transcriptsBySession: Record<string, TranscriptItem[]>;
  transcriptStatusBySession: Record<string, ResourceFetchStatus>;
  /** Infinite-scroll cursor / loading flags per session. */
  transcriptHistoryBySession: Record<string, TranscriptHistoryMeta>;
  /**
   * Attention page queue only (opened-view hydrate). Does NOT drive sidebar
   * badge — badge uses Σ project.needsAttention (§6.5).
   */
  attentionSessions: ProjectSession[];
  attentionStatus: ResourceFetchStatus;
  clis: {
    agent: string;
    displayName: string;
    installed: boolean;
    status: string;
    supportsModelSelection: boolean;
    supportsReasoningEffort: boolean;
  }[];
  clisStatus: ResourceFetchStatus;
  /**
   * ReadReceipt baseline: messageCount when user last opened a conversation.
   * Persistable (localStorage); not a business ConversationList cache.
   * unread = max(0, messageCount - baseline). Survives bootEpoch.
   */
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
   * Load Timeline messages for one conversation (`listMessages` only).
   * `quiet`: background refresh — keep prior data / older pages, skip loading flash.
   */
  loadTimeline: (
    conversationId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  /**
   * Load Inspector sessions for one conversation (`listSessions` only).
   * Non-quiet hydrate runs needsContinue resume + elevate-only transcript peeks.
   */
  loadInspector: (
    conversationId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  /**
   * Prepend one older page of conversation messages (infinite scroll).
   * Always quiet — does not flash the main loading state.
   */
  loadOlderMessages: (conversationId: string) => Promise<void>;
  loadProjectSessions: (
    projectId: string,
    opts?: { quiet?: boolean },
  ) => Promise<void>;
  loadTranscript: (
    sessionId: string,
    opts?: {
      /** Append events after the highest cached seq (live poll). */
      append?: boolean;
      /**
       * Load one page of older history before the current window (infinite scroll).
       * Always quiet — does not flash the main loading state.
       */
      older?: boolean;
      /** @deprecated Prefer infinite scroll (`older`); still supported for tools. */
      full?: boolean;
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
  /**
   * After daemon restart, reattach a suspended session. When `needsContinue` is
   * set, inject CONTINUE so a mid-turn agent keeps working. Idempotent per
   * thread for the current boot (StrictMode / double-open safe).
   */
  resumeInterruptedSession: (sessionId: string) => Promise<void>;
  /** Load attention queue across all projects (approvals / failed / suspended). */
  loadAttentionSessions: (opts?: { quiet?: boolean }) => Promise<void>;
  resolveApproval: (
    sessionId: string,
    requestId: string,
    decision: string | Record<string, unknown>,
  ) => Promise<void>;
  respondOpencodePermission: (
    sessionId: string,
    permissionId: string,
    response: string,
  ) => Promise<void>;
  respondOpencodeQuestion: (
    sessionId: string,
    questionId: string,
    answers: string[][],
  ) => Promise<void>;
  /** Mark conversation messages as read (clears unread badge). */
  markConversationRead: (conversationId: string) => void;
  clearActionError: () => void;
  sendMessage: (
    conversationId: string,
    body: string,
    messageId?: string,
    options?: { replyToMessageId?: string },
  ) => Promise<void>;
  /**
   * Retry a failed user message by reusing its original message_id (store
   * append is idempotent upsert). Patches the existing failed row in place;
   * does NOT push a new optimistic bubble.
   */
  retryFailedMessage: (
    conversationId: string,
    messageId: string,
  ) => Promise<void>;
  createConversation: (
    projectId: string,
    input: {
      title: string;
      priority?: ConversationPriority | null;
      /** Runtime agent ids to start after create (opt-in). */
      agents?: string[];
    },
  ) => Promise<string | null>;
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

/** Zustand set/get for action factories. */
export type WorkspaceSet = StoreApi<WorkspaceState>["setState"];
export type WorkspaceGet = StoreApi<WorkspaceState>["getState"];

// re-export commonly needed domain types for action modules
export type {
  Conversation,
  ConversationProgress,
  DeliveryStatus,
  Project,
  TimelineMessage,
  ConversationBoardColumn,
};
