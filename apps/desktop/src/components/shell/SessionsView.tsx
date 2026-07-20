import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowLeft,
  Bot,
  ChevronDown,
  ChevronRight,
  FileDiff,
  Loader2,
  MessageSquare,
  PanelLeftClose,
  PanelRightClose,
  PanelRightOpen,
  ShieldAlert,
} from "lucide-react";
import { agentMeta, statusMeta } from "@/lib/mock-data";
import { Avatar } from "@/components/Avatar";
import { DiffView } from "@/components/DiffView";
import { ReadView, shouldUseReadView } from "@/components/ReadView";
import { MarkdownText } from "@/components/MarkdownText";
import { StatusPill } from "@/components/StatusPill";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { formatLocalClock, formatRelative } from "@/lib/time";
import { cn } from "@/lib/utils";
import { toast } from "@/lib/toast";
import type { TranscriptItem } from "@/lib/daemon";
import { followContentKey } from "@/lib/stick-to-bottom";
import { useStickToBottom } from "@/lib/use-stick-to-bottom";
import {
  buildToolHeader,
  collapsedThinkingSummary,
  displayToolDetail,
  isDiffLike,
} from "@/lib/tool-present";
import {
  childrenOf,
  groupSessionsByConversation,
  sessionIsExecuting,
  type ConversationSessionGroup,
} from "@/lib/session-list-group";
import {
  displayPath,
  summarizeSessionFromTranscript,
  type FileChangeEntry,
} from "@/lib/session-summary";
import {
  EMPTY_TRANSCRIPT_HISTORY,
  TRANSCRIPT_AUTOFILL_SLACK_PX,
  TRANSCRIPT_PAGE_EVENTS,
  TRANSCRIPT_PREFETCH_TOP_PX,
} from "@/lib/transcript-history";
import { IncrementalText } from "@/components/IncrementalText";

/** Stable empty list for Zustand selectors (never allocate in getSnapshot). */
const EMPTY_PROJECT_SESSIONS: ProjectSession[] = [];

/**
 * Project sessions tab — parent passes projectId (and key).
 * List load is owned here; transcript load is owned by TranscriptPane.
 *
 * Left rail is conversation-grouped (Codex-style folder → runs): each
 * conversation collapses independently; sessions show live progress.
 */
/**
 * Resolve a session for the Sessions tab **within the current project only**.
 *
 * Deep-link ids may exist only in `sessionsByConversation` until the project
 * list load returns. To avoid rendering a foreign-project transcript under
 * the current project chrome during that window, we require the session's
 * `conversationId` to reference a conversation whose `projectId` matches the
 * current project (conversations carry `projectId` on the row, unlike sessions).
 * If conversations haven't loaded yet, we fall back to the project-sessions
 * list membership check (empty during load → reject).
 */
function resolveSessionForView(
  sessionId: string | null,
  projectId: string,
  projectSessions: ProjectSession[],
  sessionsByConversation: Record<string, ProjectSession[]>,
  conversationProjectById: Record<string, string>,
): ProjectSession | undefined {
  if (!sessionId || !projectId) return undefined;
  const fromProject = projectSessions.find((s) => s.id === sessionId);
  if (fromProject) return fromProject;
  for (const list of Object.values(sessionsByConversation)) {
    const hit = list.find((s) => s.id === sessionId);
    if (!hit) continue;
    // Authoritative check: conversation row carries projectId. Reject if the
    // conversation is known and belongs to a different project.
    const convProject = conversationProjectById[hit.conversationId];
    if (convProject && convProject !== projectId) continue;
    // Conversation not yet loaded — only allow if a project-scoped session
    // shares this conversationId (weak signal, but better than nothing).
    if (
      !convProject &&
      projectSessions.length > 0 &&
      !projectSessions.some((s) => s.conversationId === hit.conversationId)
    ) {
      continue;
    }
    return hit;
  }
  return undefined;
}

function sessionBelongsToProject(
  sessionId: string | null,
  projectSessions: ProjectSession[],
): boolean {
  if (!sessionId) return false;
  return projectSessions.some((s) => s.id === sessionId);
}

export function SessionsView({ projectId }: { projectId: string }) {
  const selectedSessionId = useUiStore((s) => s.selectedSessionId);
  const conversationId = useUiStore((s) => s.conversationId);
  const projectView = useUiStore((s) => s.projectView);
  const selectSession = useUiStore((s) => s.selectSession);
  const openConversation = useUiStore((s) => s.openConversation);
  const listCollapsed = useUiStore((s) => s.sessionsListCollapsed);
  const toggleSessionsList = useUiStore((s) => s.toggleSessionsList);

  // Important: never fall back to the global `projectSessions` array — it may
  // still hold the previous project's rows while the new key is empty/loading.
  const projectSessions = useWorkspaceStore(
    (s) => s.projectSessionsByProject[projectId] ?? EMPTY_PROJECT_SESSIONS,
  );
  const sessionsByConversation = useWorkspaceStore(
    (s) => s.sessionsByConversation,
  );
  const conversations = useWorkspaceStore((s) => s.conversations);
  const listStatus = useWorkspaceStore(
    (s) => s.projectSessionsStatusByProject[projectId],
  );
  const loadProjectSessions = useWorkspaceStore((s) => s.loadProjectSessions);
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const livePush = useWorkspaceStore((s) => s.livePush);

  /** Conversation ids the user has collapsed (default: all expanded). */
  const [collapsedConvIds, setCollapsedConvIds] = useState<Set<string>>(
    () => new Set(),
  );

  // conversationId → projectId, so deep-link resolution can reject a foreign
  // project's session even during the project-sessions loading race.
  const conversationProjectById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of conversations) {
      if (c.projectId) map[c.id] = c.projectId;
    }
    return map;
  }, [conversations]);

  // Merge deep-linked session into the list only if it belongs to this project.
  const displaySessions = useMemo(() => {
    if (!selectedSessionId) return projectSessions;
    if (sessionBelongsToProject(selectedSessionId, projectSessions)) {
      return projectSessions;
    }
    const fallback = resolveSessionForView(
      selectedSessionId,
      projectId,
      projectSessions,
      sessionsByConversation,
      conversationProjectById,
    );
    if (!fallback) return projectSessions;
    // Still only inject while list is empty (deep-link race), never a foreign id.
    if (projectSessions.length > 0) return projectSessions;
    return [fallback, ...projectSessions];
  }, [
    projectSessions,
    selectedSessionId,
    sessionsByConversation,
    conversationProjectById,
    projectId,
  ]);

  const groups = useMemo(
    () => groupSessionsByConversation(displaySessions),
    [displaySessions],
  );

  // Init: load project sessions for this projectId (re-run after boot wipe).
  // Quiet when we already have rows so keep-alive re-entry / remount does not
  // flash the list into "Loading…".
  useEffect(() => {
    if (source !== "daemon") return;
    const hasRows =
      (useWorkspaceStore.getState().projectSessionsByProject[projectId]
        ?.length ?? 0) > 0;
    void loadProjectSessions(projectId, { quiet: hasRows });
  }, [projectId, source, loadProjectSessions, bootEpoch]);

  // Project switch: drop foreign selection + collapse state.
  useEffect(() => {
    setCollapsedConvIds(new Set());
    // Clear selection that does not belong to the new project's loaded list.
    // While loading (empty list), leave deep-link id alone until list settles.
    const phase = listStatus?.phase ?? "idle";
    if (phase === "loading" || phase === "idle") return;
    if (
      selectedSessionId &&
      !sessionBelongsToProject(selectedSessionId, projectSessions)
    ) {
      selectSession(null);
    }
  }, [
    projectId,
    listStatus?.phase,
    projectSessions,
    selectedSessionId,
    selectSession,
  ]);

  // Auto-select a root session when nothing valid is selected.
  // Prefer the session under the conversation the user just left (Conversations tab).
  //
  // Only while the Sessions tab is active: keep-alive leaves this view mounted
  // under Conversations, and re-selecting here would undo SessionInspector's
  // "← Back to conversation" (selectSession(null)).
  useEffect(() => {
    if (projectView !== "sessions") return;
    if (selectedSessionId) {
      if (sessionBelongsToProject(selectedSessionId, displaySessions)) return;
      // Foreign or stale id — do not wait forever on empty load.
      const phase = listStatus?.phase ?? "idle";
      if (phase === "loading" || phase === "idle") return;
    }
    const fromConversation = conversationId
      ? displaySessions.find(
          (s) => s.conversationId === conversationId && !s.parentId,
        )
      : undefined;
    const firstRoot =
      fromConversation ?? groups[0]?.roots[0] ?? displaySessions[0];
    if (firstRoot) selectSession(firstRoot.id);
    else if (selectedSessionId) selectSession(null);
  }, [
    projectView,
    groups,
    displaySessions,
    selectedSessionId,
    selectSession,
    listStatus?.phase,
    conversationId,
  ]);

  // Keep the conversation of the selected session expanded.
  useEffect(() => {
    if (!selectedSessionId) return;
    const session = displaySessions.find((s) => s.id === selectedSessionId);
    if (!session) return;
    setCollapsedConvIds((prev) => {
      if (!prev.has(session.conversationId)) return prev;
      const next = new Set(prev);
      next.delete(session.conversationId);
      return next;
    });
  }, [selectedSessionId, displaySessions]);

  // Fallback list poll only without live push (manager events own live status).
  useEffect(() => {
    if (source !== "daemon" || livePush) return;
    const live = displaySessions.some(
      (s) => s.status === "running" || s.status === "needs_approval",
    );
    if (!live && listStatus?.phase !== "error") return;
    const id = window.setInterval(() => {
      void loadProjectSessions(projectId, { quiet: true });
    }, 2000);
    return () => window.clearInterval(id);
  }, [
    projectId,
    source,
    livePush,
    displaySessions,
    listStatus?.phase,
    loadProjectSessions,
  ]);

  const selected = resolveSessionForView(
    selectedSessionId,
    projectId,
    displaySessions,
    sessionsByConversation,
    conversationProjectById,
  );
  const phase = listStatus?.phase ?? "idle";
  const conversationCount = groups.length;
  const liveTotal = groups.reduce((n, g) => n + g.runningCount, 0);

  const toggleConversation = (conversationId: string) => {
    setCollapsedConvIds((prev) => {
      const next = new Set(prev);
      if (next.has(conversationId)) next.delete(conversationId);
      else next.add(conversationId);
      return next;
    });
  };

  return (
    <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
      {listCollapsed ? (
        <div className="flex w-10 shrink-0 flex-col items-center border-r border-ink/5 bg-surface pt-2.5">
          <button
            type="button"
            title="Expand sessions list"
            onClick={toggleSessionsList}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-ink-muted hover:bg-surface-hover hover:text-ink"
          >
            <Bot className="h-4 w-4" />
          </button>
        </div>
      ) : (
        <aside className="flex w-[min(300px,36vw)] min-w-[240px] max-w-[360px] shrink-0 flex-col overflow-hidden border-r border-ink/5 bg-surface">
          <div className="flex shrink-0 items-center justify-between border-b border-ink/5 px-3 py-2.5">
            <div className="min-w-0 pl-1">
              <div className="text-[13px] font-semibold text-ink">Sessions</div>
              <div className="text-[11px] text-ink-muted">
                {phase === "loading" && projectSessions.length === 0
                  ? "Loading…"
                  : conversationCount === 0
                    ? "No sessions"
                    : `${conversationCount} conversation${conversationCount === 1 ? "" : "s"} · ${projectSessions.length} session${projectSessions.length === 1 ? "" : "s"}${liveTotal > 0 ? ` · ${liveTotal} live` : ""}`}
              </div>
            </div>
            <button
              type="button"
              title="Collapse"
              onClick={toggleSessionsList}
              className="flex h-8 w-8 items-center justify-center rounded-lg text-ink-muted hover:bg-surface-hover"
            >
              <PanelLeftClose className="h-4 w-4" />
            </button>
          </div>

          <div className="scrollbar-thin min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
            {phase === "error" && projectSessions.length === 0 ? (
              <div className="flex flex-col items-center gap-2 px-2 py-8 text-center">
                <p className="text-[12px] text-rose-600">
                  {listStatus?.error ?? "Failed to load sessions"}
                </p>
                <button
                  type="button"
                  onClick={() => void loadProjectSessions(projectId)}
                  className="rounded-lg bg-ink px-3 py-1.5 text-[11px] font-semibold text-white"
                >
                  Retry
                </button>
              </div>
            ) : null}
            {groups.map((group) => (
              <ConversationSessionFolder
                key={group.conversationId}
                group={group}
                collapsed={collapsedConvIds.has(group.conversationId)}
                selectedSessionId={selectedSessionId}
                onToggle={() => toggleConversation(group.conversationId)}
                onSelectSession={selectSession}
              />
            ))}
            {phase === "loading" && projectSessions.length === 0 ? (
              <p className="px-2 py-8 text-center text-[12px] text-ink-muted">
                Loading sessions…
              </p>
            ) : null}
            {phase === "ready" && projectSessions.length === 0 ? (
              <p className="px-2 py-8 text-center text-[12px] text-ink-muted">
                No agent sessions yet. Use @agent in a conversation.
              </p>
            ) : null}
          </div>
        </aside>
      )}

      {selectedSessionId && selected ? (
        <TranscriptPane
          key={selectedSessionId}
          sessionId={selectedSessionId}
          session={selected}
          onBackToConversation={() =>
            openConversation(selected.conversationId)
          }
        />
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center bg-surface text-[13px] text-ink-muted">
          Select an agent session to view its full transcript.
        </div>
      )}
    </div>
  );
}

function ConversationSessionFolder({
  group,
  collapsed,
  selectedSessionId,
  onToggle,
  onSelectSession,
}: {
  group: ConversationSessionGroup;
  collapsed: boolean;
  selectedSessionId: string | null;
  onToggle: () => void;
  onSelectSession: (id: string) => void;
}) {
  const hasSelected = group.sessions.some((s) => s.id === selectedSessionId);

  return (
    <div
      className={cn(
        "rounded-xl",
        hasSelected && !collapsed ? "bg-surface-muted/40" : null,
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        className={cn(
          "flex w-full items-center gap-1.5 rounded-xl px-2 py-2 text-left transition-colors",
          "hover:bg-surface-hover",
          hasSelected && collapsed ? "bg-surface-muted/60" : null,
        )}
        aria-expanded={!collapsed}
      >
        {collapsed ? (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        ) : (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        )}
        <MessageSquare className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        <span
          className="min-w-0 flex-1 truncate text-[12.5px] font-semibold text-ink"
          title={group.title}
        >
          {group.title}
        </span>
        {group.runningCount > 0 ? (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-amber-100 px-1.5 py-0.5 text-[10px] font-medium text-amber-800">
            <Loader2 className="h-3 w-3 animate-spin" />
            {group.runningCount}
          </span>
        ) : null}
        {group.attentionCount > 0 && group.runningCount === 0 ? (
          <span className="shrink-0 rounded-full bg-rose-100 px-1.5 py-0.5 text-[10px] font-medium text-rose-800">
            {group.attentionCount}
          </span>
        ) : null}
        <span className="shrink-0 text-[10px] tabular-nums text-ink-muted">
          {group.sessions.length}
        </span>
      </button>

      {!collapsed ? (
        <div className="space-y-0.5 pb-1 pl-1 pr-0.5">
          {group.roots.length === 0 ? (
            <p className="px-3 py-2 text-[11px] text-ink-muted">
              No top-level sessions
            </p>
          ) : (
            group.roots.map((session) => (
              <SessionTreeRow
                key={session.id}
                session={session}
                all={group.sessions}
                depth={0}
                selectedId={selectedSessionId}
                onSelect={onSelectSession}
              />
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}

function SessionTreeRow({
  session,
  all,
  depth,
  selectedId,
  onSelect,
}: {
  session: ProjectSession;
  all: ProjectSession[];
  depth: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const children = childrenOf(session.id, all);
  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const selected = selectedId === session.id;
  const executing = sessionIsExecuting(session.status);
  const status = statusMeta[session.status] ?? statusMeta.idle;
  const when = session.lastTsMs ? formatRelative(session.lastTsMs) : undefined;

  return (
    <div>
      <button
        type="button"
        onClick={() => onSelect(session.id)}
        style={{ paddingLeft: 8 + depth * 12 }}
        className={cn(
          "flex w-full gap-2 rounded-lg py-2 pr-2 text-left transition-colors",
          selected
            ? "bg-surface-muted shadow-panel ring-1 ring-ink/5"
            : "hover:bg-surface-hover",
        )}
      >
        <div className="relative shrink-0">
          <Avatar
            name={meta?.label ?? session.agent}
            tone={meta?.tone ?? "slate"}
            size="sm"
          />
          {executing ? (
            <span
              className="absolute -bottom-0.5 -right-0.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-surface ring-1 ring-ink/10"
              title="Executing"
            >
              <Loader2 className="h-2.5 w-2.5 animate-spin text-amber-600" />
            </span>
          ) : null}
        </div>
        <div className="min-w-0 flex-1">
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-2">
            <span className="truncate text-[12.5px] font-semibold text-ink">
              {meta?.label ?? session.agent}{" "}
              <span className="font-mono text-[10.5px] font-normal text-ink-muted">
                #{session.shortId}
              </span>
            </span>
            {when ? (
              <span className="text-[10.5px] tabular-nums text-ink-muted">
                {when}
              </span>
            ) : null}
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
            <span
              className={cn(
                "inline-flex max-w-full items-center gap-1 truncate rounded-full px-1.5 py-0.5 text-[10px] font-medium",
                status.pill,
              )}
            >
              {executing ? (
                <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" />
              ) : (
                <span
                  className={cn("h-1.5 w-1.5 shrink-0 rounded-full", status.dot)}
                />
              )}
              {status.label}
            </span>
            {session.parentId ? (
              <span className="truncate text-[10px] text-ink-muted">
                subagent
              </span>
            ) : null}
          </div>
          {session.summary ? (
            <p
              className="mt-0.5 line-clamp-1 text-[11px] leading-snug text-ink-muted"
              title={session.summary}
            >
              {session.summary}
            </p>
          ) : null}
        </div>
      </button>
      {children.map((child) => (
        <SessionTreeRow
          key={child.id}
          session={child}
          all={all}
          depth={depth + 1}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}

/**
 * Stable empty snapshot for Zustand selectors.
 * `?? []` inside a selector returns a new array every getSnapshot call, which
 * trips React useSyncExternalStore into "Maximum update depth exceeded".
 */
const EMPTY_TRANSCRIPT: TranscriptItem[] = [];

function TranscriptPane({
  sessionId,
  session,
  onBackToConversation,
}: {
  sessionId: string;
  session: ProjectSession;
  onBackToConversation?: () => void;
}) {
  const resolveApproval = useWorkspaceStore((s) => s.resolveApproval);
  const respondOpencodePermission = useWorkspaceStore(
    (s) => s.respondOpencodePermission,
  );
  const respondOpencodeQuestion = useWorkspaceStore(
    (s) => s.respondOpencodeQuestion,
  );
  const loadTranscript = useWorkspaceStore((s) => s.loadTranscript);
  const items = useWorkspaceStore(
    (s) => s.transcriptsByThread[sessionId] ?? EMPTY_TRANSCRIPT,
  );
  const status = useWorkspaceStore(
    (s) => s.transcriptStatusByThread[sessionId],
  );
  // Primitive fields only — never `?? newObject()` in a Zustand selector
  // (that re-allocates every getSnapshot and hits max update depth).
  const hasOlder = useWorkspaceStore(
    (s) => s.transcriptHistoryByThread[sessionId]?.hasOlder ?? false,
  );
  const loadingOlder = useWorkspaceStore(
    (s) => s.transcriptHistoryByThread[sessionId]?.loadingOlder ?? false,
  );
  const firstLoadedStartSeq = useWorkspaceStore(
    (s) =>
      s.transcriptHistoryByThread[sessionId]?.firstLoadedStartSeq ??
      EMPTY_TRANSCRIPT_HISTORY.firstLoadedStartSeq,
  );
  const source = useWorkspaceStore((s) => s.source);
  const livePush = useWorkspaceStore((s) => s.livePush);
  const [approving, setApproving] = useState<string | null>(null);
  const [summaryOpen, setSummaryOpen] = useState(true);

  /** Preserve viewport when older pages are prepended (no jump). */
  const scrollAnchorRef = useRef<{
    height: number;
    top: number;
  } | null>(null);
  /** Prevent overlapping older fetches (state lag vs double effect fire). */
  const olderInFlightRef = useRef(false);

  const contentKey = useMemo(() => followContentKey(items), [items]);
  const { scrollRef, contentRef, following, jumpToLatest, markProgrammatic } =
    useStickToBottom({
      contentKey,
      resetKey: sessionId,
    });

  const summary = useMemo(
    () => summarizeSessionFromTranscript(items),
    [items],
  );

  const liveStreaming =
    session.status === "running" || session.status === "needs_approval";
  const lastStreamableId = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i--) {
      const k = items[i]!.kind;
      if (k === "assistant" || k === "text" || k === "reasoning" || k === "user") {
        return items[i]!.id;
      }
    }
    return null;
  }, [items]);

  const loadOlder = useCallback(async () => {
    if (source !== "daemon") return;
    if (!hasOlder || loadingOlder || olderInFlightRef.current) return;
    olderInFlightRef.current = true;
    const el = scrollRef.current;
    if (el) {
      scrollAnchorRef.current = {
        height: el.scrollHeight,
        top: el.scrollTop,
      };
    }
    try {
      await loadTranscript(sessionId, {
        older: true,
        quiet: true,
        tailWindow: TRANSCRIPT_PAGE_EVENTS,
        approvalStatusPolicy: "sync",
      });
    } finally {
      olderInFlightRef.current = false;
    }
  }, [source, hasOlder, loadingOlder, loadTranscript, sessionId, scrollRef]);

  // After older items prepend, keep the same messages under the viewport.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const anchor = scrollAnchorRef.current;
    if (!el || !anchor) return;
    const delta = el.scrollHeight - anchor.height;
    if (delta > 0) {
      // Do not let stick-to-bottom treat this as a user scroll (re-follow thrash).
      markProgrammatic(100);
      el.scrollTop = anchor.top + delta;
    }
    scrollAnchorRef.current = null;
  }, [items, scrollRef, markProgrammatic]);

  // Init / reopen: first open loads the tail; when cache exists only append
  // new events (quiet) so switching tabs does not wipe older pages or flash
  // the loading skeleton.
  useEffect(() => {
    if (source !== "daemon") return;
    olderInFlightRef.current = false;
    const cached =
      useWorkspaceStore.getState().transcriptsByThread[sessionId] ?? [];
    const hasCache = cached.length > 0;
    void loadTranscript(sessionId, {
      tailWindow: TRANSCRIPT_PAGE_EVENTS,
      quiet: hasCache,
      append: hasCache,
      approvalStatusPolicy: "sync",
    });
  }, [sessionId, source, loadTranscript]);

  // Silent backfill only while stick-to-bottom is active (opening a session /
  // sparse tail). Never autofill while the user is reading older history —
  // that used to race with manual scroll and cause viewport thrash.
  useEffect(() => {
    if (source !== "daemon") return;
    if (!following) return;
    if (status?.phase !== "ready") return;
    if (!hasOlder || loadingOlder) return;
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollHeight <= el.clientHeight + TRANSCRIPT_AUTOFILL_SLACK_PX) {
      void loadOlder();
    }
  }, [
    source,
    following,
    status?.phase,
    hasOlder,
    loadingOlder,
    firstLoadedStartSeq,
    items.length,
    loadOlder,
    scrollRef,
  ]);

  // Prefetch older when the user approaches the top (manual history browse).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const onScroll = () => {
      if (following) return;
      if (el.scrollTop > TRANSCRIPT_PREFETCH_TOP_PX) return;
      void loadOlder();
    };

    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [sessionId, loadOlder, scrollRef, following]);

  // Fallback append poll only without live push (ingest frames own live stream).
  useEffect(() => {
    if (source !== "daemon" || livePush) return;
    const live =
      session.status === "running" || session.status === "needs_approval";
    if (!live && status?.phase !== "error") return;
    const id = window.setInterval(() => {
      void loadTranscript(sessionId, {
        append: true,
        quiet: true,
        approvalStatusPolicy: "sync",
      });
    }, 2000);
    return () => window.clearInterval(id);
  }, [
    sessionId,
    session.status,
    source,
    livePush,
    status?.phase,
    loadTranscript,
  ]);

  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const phase = status?.phase ?? "idle";
  const hasCache = items.length > 0;

  return (
    <section className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface">
      <header className="flex shrink-0 items-center justify-between gap-3 border-b border-ink/5 px-4 py-3 sm:px-5">
        <div className="min-w-0 flex-1">
          {onBackToConversation ? (
            <button
              type="button"
              onClick={onBackToConversation}
              className="mb-1.5 inline-flex items-center gap-1 text-[12px] font-medium text-ink-muted hover:text-ink"
            >
              <ArrowLeft className="h-3.5 w-3.5" />
              Back to conversation
            </button>
          ) : null}
          <div className="flex min-w-0 items-center gap-2.5">
            <Avatar
              name={meta?.label ?? session.agent}
              tone={meta?.tone ?? "slate"}
            />
            <div className="min-w-0">
              <h2 className="truncate text-[15px] font-semibold tracking-tight text-ink">
                {meta?.label ?? session.agent}{" "}
                <span className="font-mono text-[12px] font-normal text-ink-muted">
                  #{session.shortId}
                </span>
                {!following ? (
                  <span className="ml-2 text-[11px] font-normal text-ink-muted">
                    [manual scroll]
                  </span>
                ) : null}
              </h2>
              {session.conversationTitle ? (
                <div className="mt-1 flex min-w-0 max-w-[280px] items-center gap-1 truncate text-[12px] text-ink-muted">
                  <MessageSquare className="h-3 w-3 shrink-0" />
                  <span className="truncate" title={session.conversationTitle}>
                    {session.conversationTitle}
                  </span>
                </div>
              ) : null}
            </div>
          </div>
        </div>
        <button
          type="button"
          title={summaryOpen ? "Hide session summary" : "Show session summary"}
          onClick={() => setSummaryOpen((v) => !v)}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted hover:bg-surface-hover hover:text-ink"
        >
          {summaryOpen ? (
            <PanelRightClose className="h-4 w-4" />
          ) : (
            <PanelRightOpen className="h-4 w-4" />
          )}
        </button>
      </header>

      {/*
        Transcript + summary side-by-side.
        Use flex + flex-basis:0 (not grid + absolute inset) so the scroll
        pane always gets a definite max height in Tauri/WKWebView.
      */}
      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <div
            ref={scrollRef}
            className="scrollbar-thin min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-y-contain px-4 py-4 sm:px-5"
            style={{ flex: "1 1 0%" }}
          >
            <div ref={contentRef} className="mx-auto max-w-3xl space-y-2.5 pb-8">
              {/* Tiny non-blocking marker at top while older pages stream in */}
              {loadingOlder ? (
                <div className="flex justify-center py-1" aria-hidden>
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-ink-muted/50" />
                </div>
              ) : null}
              {phase === "loading" && !hasCache ? (
                <p className="py-12 text-center text-[13px] text-ink-muted">
                  Loading transcript…
                </p>
              ) : phase === "error" && !hasCache ? (
                <div className="flex flex-col items-center gap-3 py-12 text-center">
                  <p className="text-[13px] text-rose-600">
                    {status?.error ?? "Failed to load transcript"}
                  </p>
                  <button
                    type="button"
                    onClick={() =>
                      void loadTranscript(sessionId, {
                        tailWindow: TRANSCRIPT_PAGE_EVENTS,
                        approvalStatusPolicy: "sync",
                      })
                    }
                    className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white"
                  >
                    Retry
                  </button>
                </div>
              ) : items.length === 0 ? (
                <p className="py-12 text-center text-[13px] text-ink-muted">
                  No transcript events yet. They appear as the agent runs.
                </p>
              ) : (
                items.map((item) => (
                  <TranscriptItemView
                    key={item.id}
                    item={item}
                    streaming={
                      liveStreaming &&
                      item.id === lastStreamableId &&
                      (item.kind === "assistant" ||
                        item.kind === "text" ||
                        item.kind === "reasoning")
                    }
                    approving={approving === item.requestId}
                    onUserAction={
                      item.requestId &&
                      (item.kind === "approval" || item.kind === "question")
                        ? async (action) => {
                            setApproving(item.requestId!);
                            try {
                              await handleUserAction(
                                session.id,
                                item,
                                action,
                                {
                                  resolveApproval,
                                  respondOpencodePermission,
                                  respondOpencodeQuestion,
                                },
                              );
                              if (
                                action.type === "decision" &&
                                (item.kind === "approval" ||
                                  item.kind === "question")
                              ) {
                                toast.success(
                                  action.decision === "approve" ||
                                    action.decision === "allow"
                                    ? "Approved"
                                    : action.decision === "deny" ||
                                        action.decision === "abandon"
                                      ? "Denied"
                                      : `Decision: ${action.decision}`,
                                );
                              } else if (action.type === "cancel") {
                                toast.info("Cancelled");
                              }
                            } catch (e) {
                              toast.error(
                                "Action failed",
                                e instanceof Error ? e.message : String(e),
                              );
                              throw e;
                            } finally {
                              setApproving(null);
                            }
                          }
                        : undefined
                    }
                  />
                ))
              )}
            </div>
          </div>

          {!following ? (
            <div className="pointer-events-none absolute inset-x-0 bottom-4 z-10 flex justify-center">
              <button
                type="button"
                onClick={jumpToLatest}
                className="pointer-events-auto inline-flex items-center gap-1.5 rounded-full border border-ink/10 bg-surface px-3.5 py-1.5 text-[12px] font-medium text-ink shadow-lg hover:bg-surface-muted"
              >
                <ArrowDown className="h-3.5 w-3.5" />
                Jump to latest
              </button>
            </div>
          ) : null}
        </div>

        {summaryOpen ? (
          <SessionSummaryPanel session={session} summary={summary} />
        ) : null}
      </div>
    </section>
  );
}

function SessionSummaryPanel({
  session,
  summary,
}: {
  session: ProjectSession;
  summary: ReturnType<typeof summarizeSessionFromTranscript>;
}) {
  return (
    <aside className="flex min-h-0 w-[min(280px,32vw)] min-w-[220px] max-w-[320px] shrink-0 flex-col self-stretch overflow-hidden border-l border-ink/5 bg-surface">
      <div className="flex shrink-0 items-center gap-2 border-b border-ink/5 px-3 py-2.5">
        <FileDiff className="h-3.5 w-3.5 text-ink-muted" />
        <div className="min-w-0 flex-1">
          <div className="text-[12.5px] font-semibold text-ink">Summary</div>
          <div className="text-[10.5px] text-ink-muted">
            Session stats from tools
          </div>
        </div>
      </div>

      <div
        className="scrollbar-thin min-h-0 flex-1 space-y-4 overflow-y-auto overscroll-contain px-3 py-3"
        style={{ flex: "1 1 0%" }}
      >
        <section>
          <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-ink-muted">
            Status
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <StatusPill status={session.status} />
            {summary.pendingEdits > 0 ? (
              <span className="inline-flex items-center gap-1 text-[11px] text-amber-800">
                <Loader2 className="h-3 w-3 animate-spin" />
                {summary.pendingEdits} edit
                {summary.pendingEdits === 1 ? "" : "s"} in flight
              </span>
            ) : null}
          </div>
        </section>

        <section>
          <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-ink-muted">
            Activity
          </div>
          <dl className="grid grid-cols-2 gap-x-2 gap-y-1.5 text-[12px]">
            <dt className="text-ink-muted">Tools</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.toolCallCount}
            </dd>
            <dt className="text-ink-muted">Edits</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.editCallCount}
            </dd>
            <dt className="text-ink-muted">Files</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.files.length}
            </dd>
            <dt className="text-ink-muted">Lines</dt>
            <dd className="text-right font-mono text-[11px] tabular-nums">
              {summary.totalDel > 0 || summary.totalAdd > 0 ? (
                <>
                  <span className="text-rose-700">-{summary.totalDel}</span>
                  <span className="text-ink-muted"> / </span>
                  <span className="text-emerald-700">+{summary.totalAdd}</span>
                </>
              ) : (
                <span className="text-ink-muted">—</span>
              )}
            </dd>
          </dl>
          <p className="mt-2 text-[10.5px] leading-snug text-ink-muted">
            Token usage is not shown yet (CLI formats differ; not in unified
            projection).
          </p>
        </section>

        <section>
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-[10px] font-semibold uppercase tracking-wide text-ink-muted">
              Files changed
            </span>
            {summary.files.length > 0 ? (
              <span className="text-[10px] tabular-nums text-ink-muted">
                {summary.files.length}
              </span>
            ) : null}
          </div>
          {summary.files.length === 0 ? (
            <p className="rounded-lg bg-surface-muted/60 px-2.5 py-3 text-[11.5px] leading-snug text-ink-muted">
              No file edits in this transcript yet. Edit tools
              (write / search_replace / apply_patch …) appear here with{" "}
              <span className="font-mono">-N +M</span> when available.
            </p>
          ) : (
            <ul className="space-y-1">
              {summary.files.map((file) => (
                <FileChangeRow key={file.path} file={file} />
              ))}
            </ul>
          )}
        </section>
      </div>
    </aside>
  );
}

function FileChangeRow({ file }: { file: FileChangeEntry }) {
  const short = displayPath(file.path);
  return (
    <li
      className={cn(
        "rounded-lg px-2 py-1.5 font-mono text-[11px] leading-snug",
        file.failed
          ? "bg-rose-50/80 text-rose-900"
          : "bg-surface-muted/50 text-ink-secondary",
      )}
      title={file.path}
    >
      <div className="break-all text-ink">{short}</div>
      <div className="mt-0.5 flex items-center gap-2 tabular-nums">
        {file.del > 0 || file.add > 0 ? (
          <>
            <span className="text-rose-700">-{file.del}</span>
            <span className="text-emerald-700">+{file.add}</span>
          </>
        ) : (
          <span className="text-ink-muted">
            {file.failed ? "failed" : file.ok ? "touched" : "pending…"}
          </span>
        )}
      </div>
    </li>
  );
}

type UserAction =
  | { type: "decision"; decision: string }
  | { type: "answers"; answers: string[][] }
  | { type: "cancel" };

async function handleUserAction(
  threadId: string,
  item: TranscriptItem,
  action: UserAction,
  apis: {
    resolveApproval: (
      threadId: string,
      requestId: string,
      decision: string | Record<string, unknown>,
    ) => Promise<void>;
    respondOpencodePermission: (
      threadId: string,
      permissionId: string,
      response: string,
    ) => Promise<void>;
    respondOpencodeQuestion: (
      threadId: string,
      questionId: string,
      answers: string[][],
    ) => Promise<void>;
  },
) {
  const requestId = item.requestId;
  if (!requestId) return;
  const method = item.approvalMethod ?? "";

  if (method === "opencode/permission") {
    const token =
      action.type === "decision" &&
      (action.decision === "approve" ||
        action.decision === "allow" ||
        action.decision === "yes")
        ? (item.approveResponse ?? "accept")
        : (item.declineResponse ?? "reject");
    await apis.respondOpencodePermission(threadId, requestId, token);
    return;
  }

  if (method === "opencode/question") {
    if (action.type === "cancel") {
      await apis.respondOpencodeQuestion(threadId, requestId, [[]]);
      return;
    }
    if (action.type === "answers") {
      await apis.respondOpencodeQuestion(threadId, requestId, action.answers);
      return;
    }
    if (action.type === "decision") {
      await apis.respondOpencodeQuestion(threadId, requestId, [
        [action.decision],
      ]);
    }
    return;
  }

  if (method === "x.ai/ask_user_question") {
    if (action.type === "cancel") {
      await apis.resolveApproval(threadId, requestId, {
        outcome: "cancelled",
      });
      return;
    }
    if (action.type === "answers") {
      const map: Record<string, string[]> = {};
      action.answers.forEach((a, i) => {
        if (a.length) map[String(i)] = a;
      });
      await apis.resolveApproval(threadId, requestId, {
        outcome: "accepted",
        answers: map,
      });
      return;
    }
    if (action.type === "decision") {
      await apis.resolveApproval(threadId, requestId, {
        outcome: "accepted",
        answers: { "0": [action.decision] },
      });
    }
    return;
  }

  // Plan / ACP permission / generic approval.
  if (action.type === "decision") {
    await apis.resolveApproval(threadId, requestId, action.decision);
  } else if (action.type === "cancel") {
    await apis.resolveApproval(threadId, requestId, "deny");
  }
}

function ApprovalModal({
  item,
  isPlan,
  open,
  approving,
  onClose,
  onUserAction,
}: {
  item: TranscriptItem;
  isPlan: boolean;
  open: boolean;
  approving?: boolean;
  onClose: () => void;
  onUserAction?: (action: UserAction) => void | Promise<void>;
}) {
  const detail = item.detail?.trim() ? item.detail : null;
  // Plans can be multi‑10KB markdown; windowed display avoids one-shot paint.
  // Other approval details stay small (assembler already caps ~2KB).
  const useIncremental = isPlan && Boolean(detail) && detail!.length > 4_000;
  const options = item.options ?? [];
  const isQuestion =
    item.kind === "question" ||
    item.approvalMethod === "opencode/question" ||
    item.approvalMethod === "x.ai/ask_user_question";

  const runAction = async (action: UserAction) => {
    try {
      // Success toast is owned by the transcript onUserAction callback.
      await onUserAction?.(action);
      onClose();
    } catch (e) {
      // onUserAction may already toast; still close only on success.
      if (e) {
        /* keep open for retry */
      }
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent
        hideClose
        className="flex max-h-[min(85vh,720px)] w-full max-w-2xl flex-col gap-0 overflow-hidden p-0 sm:rounded-2xl"
        aria-describedby={undefined}
      >
        <DialogHeader className="shrink-0 space-y-1.5 pr-12">
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="h-4 w-4 shrink-0 text-rose-600" />
            {item.title ?? "Approval required"}
          </DialogTitle>
          <DialogDescription className="whitespace-pre-wrap text-left">
            {item.text}
          </DialogDescription>
          <button
            type="button"
            onClick={onClose}
            className="absolute right-4 top-4 rounded-lg px-2 py-1 text-[12px] font-medium text-ink-muted transition-colors duration-150 hover:bg-surface-muted hover:text-ink"
          >
            Close
          </button>
        </DialogHeader>
        {detail ? (
          useIncremental ? (
            <IncrementalText text={detail} className="min-h-0 px-5 py-4" />
          ) : (
            <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-5 py-4">
              <pre className="whitespace-pre-wrap font-mono text-[12.5px] leading-relaxed text-ink-secondary">
                {detail}
              </pre>
            </div>
          )
        ) : isQuestion && options.length > 0 ? (
          <div className="scrollbar-thin min-h-0 flex-1 space-y-2 overflow-y-auto px-5 py-4">
            {options.map((opt) => (
              <button
                key={opt.label}
                type="button"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: opt.label });
                }}
                className="flex w-full flex-col rounded-xl border border-ink/10 bg-white px-3.5 py-2.5 text-left transition-colors duration-150 hover:border-ink/25 hover:bg-surface-muted/60 disabled:opacity-50"
              >
                <span className="text-[13px] font-semibold text-ink">
                  {opt.label}
                </span>
                {opt.description ? (
                  <span className="mt-0.5 text-[12px] text-ink-muted">
                    {opt.description}
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        ) : (
          <div className="min-h-0 flex-1 px-5 py-4">
            <p className="text-[13px] text-ink-muted">
              {isQuestion
                ? "Pick an option above or cancel."
                : "No additional detail."}
            </p>
          </div>
        )}
        {onUserAction ? (
          <DialogFooter className="shrink-0">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={approving}
              onClick={() => {
                void runAction(
                  isQuestion
                    ? { type: "cancel" }
                    : {
                        type: "decision",
                        decision: isPlan ? "abandon" : "deny",
                      },
                );
              }}
            >
              {isPlan ? "Abandon" : isQuestion ? "Cancel" : "Deny"}
            </Button>
            {isPlan ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: "revise" });
                }}
              >
                Request changes
              </Button>
            ) : null}
            {!isQuestion ? (
              <Button
                type="button"
                size="sm"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: "approve" });
                }}
              >
                {isPlan ? "Approve plan" : "Allow"}
              </Button>
            ) : null}
          </DialogFooter>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

/**
 * Grok / TUI AgentDetail-style transcript row (not messenger bubbles).
 */
function TranscriptItemView({
  item,
  streaming,
  onUserAction,
  approving,
}: {
  item: TranscriptItem;
  streaming?: boolean;
  onUserAction?: (action: UserAction) => void | Promise<void>;
  approving?: boolean;
}) {
  const time = item.tsMs ? formatLocalClock(item.tsMs) : "";
  const [open, setOpen] = useState(Boolean(streaming));
  const [planOpen, setPlanOpen] = useState(false);

  // Stream start re-opens thinking (TUI default expand while streaming).
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  if (item.kind === "approval" || item.kind === "question") {
    const isPlan = item.approvalMethod === "x.ai/exit_plan_mode";
    const isQuestion = item.kind === "question";
    return (
      <>
        <div className="rounded-xl border border-rose-200/80 bg-rose-50/80 p-3">
          <div className="flex items-start gap-2.5">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-rose-600" />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-rose-900">
                {item.title ??
                  (isQuestion ? "Question" : "Approval required")}
              </div>
              <p className="mt-1 whitespace-pre-wrap text-[12.5px] leading-snug text-rose-900/80">
                {item.text}
              </p>
              {isQuestion && item.options && item.options.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {item.options.map((opt) => (
                    <button
                      key={opt.label}
                      type="button"
                      disabled={approving}
                      onClick={() => {
                        void onUserAction?.({
                          type: "decision",
                          decision: opt.label,
                        });
                      }}
                      className="rounded-lg border border-rose-300/80 bg-white px-2.5 py-1 text-[12px] font-medium text-rose-900 hover:bg-rose-50 disabled:opacity-50"
                    >
                      {opt.label}
                    </button>
                  ))}
                  <button
                    type="button"
                    disabled={approving}
                    onClick={() => {
                      void onUserAction?.({ type: "cancel" });
                    }}
                    className="rounded-lg px-2.5 py-1 text-[12px] font-medium text-rose-700/80 hover:bg-rose-100/60 disabled:opacity-50"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <div className="mt-2.5 flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setPlanOpen(true)}
                    className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white hover:bg-ink/90"
                  >
                    {isPlan ? "View plan" : "View details"}
                  </button>
                  {time ? (
                    <span className="text-[11px] text-ink-muted">{time}</span>
                  ) : null}
                </div>
              )}
            </div>
          </div>
        </div>
        <ApprovalModal
          item={item}
          isPlan={isPlan}
          open={planOpen}
          approving={approving}
          onClose={() => setPlanOpen(false)}
          onUserAction={onUserAction}
        />
      </>
    );
  }

  if (item.kind === "user") {
    return (
      <div className="text-[13.5px] leading-relaxed text-ink">
        <span className="select-none text-ink-muted">❯ </span>
        <span className="whitespace-pre-wrap break-words">{item.text}</span>
        {streaming ? (
          <span className="ml-0.5 inline-block animate-pulse text-ink-muted">
            █
          </span>
        ) : null}
      </div>
    );
  }

  if (item.kind === "assistant" || item.kind === "text") {
    return <MarkdownText text={item.text} streaming={streaming} />;
  }

  if (item.kind === "reasoning") {
    const header = streaming ? "Thinking…" : "Thought";
    const preview = collapsedThinkingSummary(item.text, 100);
    return (
      <div className="text-[12.5px] leading-relaxed">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex w-full items-center gap-1.5 text-left text-ink-secondary hover:text-ink"
        >
          {open ? (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          )}
          <span className="font-medium text-ink-muted">{header}</span>
          {!open && preview ? (
            <span className="min-w-0 truncate text-ink-muted/80">{preview}</span>
          ) : null}
        </button>
        {open ? (
          <div className="mt-1 space-y-0.5 border-l-2 border-ink/10 pl-3 text-ink-secondary">
            {item.text.split("\n").map((line, i) => (
              <div key={i} className="flex gap-2">
                <span className="select-none text-ink-muted/50">│</span>
                <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
                  {line || "\u00a0"}
                </span>
              </div>
            ))}
            {streaming ? (
              <div className="flex gap-2">
                <span className="select-none text-ink-muted/50">│</span>
                <span className="animate-pulse text-ink-muted">█</span>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    );
  }

  if (
    item.kind === "tool" ||
    item.kind === "tool_result" ||
    item.kind === "tool_error"
  ) {
    const header = buildToolHeader({
      toolName: item.title ?? "tool",
      target: item.text,
      kind: item.kind,
      detail: item.detail,
    });
    // Strip SGR color codes from bash/CLI tool bodies (Grok ACP raw bytes).
    const detail = displayToolDetail(item.detail).trim();
    const expandable = Boolean(detail);
    // Only real patches (not tool-args JSON) use DiffView.
    const showDiff = detail.length > 0 && isDiffLike(detail);
    // Grok read_file emits `N→content` markers for the model; render as gutter.
    const showRead = shouldUseReadView({
      toolName: item.title ?? "tool",
      detail,
      isDiff: showDiff,
    });
    return (
      <div className="text-[12.5px] leading-snug">
        <button
          type="button"
          disabled={!expandable}
          onClick={() => expandable && setOpen((v) => !v)}
          className={cn(
            "flex w-full max-w-full items-baseline gap-1.5 text-left",
            expandable ? "cursor-pointer hover:opacity-90" : "cursor-default",
          )}
        >
          {expandable ? (
            open ? (
              <ChevronDown className="mt-0.5 h-3 w-3 shrink-0 text-ink-muted" />
            ) : (
              <ChevronRight className="mt-0.5 h-3 w-3 shrink-0 text-ink-muted" />
            )
          ) : (
            <span className="inline-block w-3 shrink-0" />
          )}
          <span
            className={cn(
              "shrink-0 font-medium",
              header.failed ? "text-rose-700" : "text-ink-secondary",
            )}
          >
            {header.verb}
          </span>
          <span
            className={cn(
              "min-w-0 truncate font-mono text-[12px]",
              header.failed ? "text-rose-800/90" : "text-ink",
            )}
            title={header.target}
          >
            {header.target}
          </span>
          {header.running ? (
            <span className="shrink-0 text-ink-muted">…</span>
          ) : null}
          {header.failed ? (
            <span className="shrink-0 text-rose-600">failed</span>
          ) : null}
          {header.diffstat && !header.running && !header.failed ? (
            <span className="shrink-0 tabular-nums">
              <span className="text-emerald-700">+{header.diffstat.add}</span>
              <span className="text-ink-muted">/</span>
              <span className="text-rose-600">-{header.diffstat.del}</span>
            </span>
          ) : null}
          {time ? (
            <span className="ml-auto shrink-0 text-[11px] tabular-nums text-ink-muted">
              {time}
            </span>
          ) : null}
        </button>
        {open && detail ? (
          showDiff ? (
            <DiffView text={detail} />
          ) : showRead ? (
            <ReadView text={detail} />
          ) : (
            <pre className="mt-1 max-h-72 overflow-auto rounded-lg border border-ink/5 bg-surface-muted/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-ink-secondary whitespace-pre-wrap">
              {detail}
            </pre>
          )
        ) : null}
      </div>
    );
  }

  if (item.kind === "error") {
    return (
      <div className="rounded-lg border border-rose-200/80 bg-rose-50/70 px-3 py-2 text-[13px] text-rose-900">
        {item.text}
      </div>
    );
  }

  if (item.kind === "status" || item.kind === "system") {
    return <div className="text-[12px] text-ink-muted">{item.text}</div>;
  }

  return (
    <div className="text-[11px] text-ink-muted">
      {item.title ?? item.kind}
      {item.text ? ` · ${item.text.slice(0, 120)}` : ""}
    </div>
  );
}
