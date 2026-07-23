import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ArrowDown,
  ArrowLeft,
  Bot,
  ChevronDown,
  ChevronRight,
  FileDiff,
  Loader2,
  MessageSquare,
  PanelRightClose,
  PanelRightOpen,
  ShieldAlert,
} from "lucide-react";
import { agentMeta } from "@/shared/lib/mock-data";
import { Avatar } from "@/shared/ui/Avatar";
import { DiffView } from "@/shared/ui/DiffView";
import { ReadView, shouldUseReadView } from "@/shared/ui/ReadView";
import { MarkdownText } from "@/shared/ui/MarkdownText";
import { StatusPill } from "@/shared/ui/StatusPill";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";
import { toast } from "@/shared/lib/toast";
import type { TranscriptItem } from "@/shared/lib/daemon";
import { followContentKey } from "@/shared/lib/stick-to-bottom";
import { useStickToBottom } from "@/shared/lib/use-stick-to-bottom";
import {
  captureItemScrollAnchor,
  queryScrollItem,
  restoreItemScrollAnchor,
  type ItemScrollAnchor,
} from "@/shared/lib/scroll-restore";
import {
  buildToolHeader,
  collapsedThinkingSummary,
  displayToolDetail,
  isDiffLike,
} from "@/shared/lib/tool-present";
import { groupSessionsByConversation } from "@/shared/lib/session-list-group";
import {
  displayPath,
  summarizeSessionFromTranscript,
  type FileChangeEntry,
} from "@/shared/lib/session-summary";
import {
  EMPTY_TRANSCRIPT_HISTORY,
  TRANSCRIPT_AUTOFILL_SLACK_PX,
  TRANSCRIPT_PAGE_EVENTS,
} from "@/shared/lib/transcript-history";
import {
  itemShowsStreamingCursor,
  streamingTailItemId,
} from "@/shared/lib/transcript-streaming";
import { IncrementalText } from "@/shared/ui/IncrementalText";
import { SessionListPane } from "./SessionListPane";

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

  // SessionList for this project only (no global projectSessions mirror).
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

  // Load project sessions when:
  // - projectId / bootEpoch changes, or
  // - user switches to Sessions tab (keep-alive may still be mounted with a
  //   stale list after agents started under other conversations).
  // Quiet when we already have rows so remount does not flash "Loading…".
  useEffect(() => {
    if (source !== "daemon") return;
    // Keep-alive under Conversations: do not spam list_project_sessions on every
    // conversation click — only hydrate while Sessions tab is active (or first
    // mount when already on Sessions).
    if (projectView !== "sessions") return;
    const hasRows =
      (useWorkspaceStore.getState().projectSessionsByProject[projectId]
        ?.length ?? 0) > 0;
    void loadProjectSessions(projectId, { quiet: hasRows });
  }, [projectId, source, loadProjectSessions, bootEpoch, projectView]);

  // Project switch: reset folder collapse. (selectProject already clears
  // selectedSessionId; do not re-clear here.)
  useEffect(() => {
    setCollapsedConvIds(new Set());
  }, [projectId]);

  // Auto-select a root session when nothing valid is selected.
  // Prefer the session under the conversation the user just left (Conversations tab).
  //
  // Only while the Sessions tab is active: keep-alive leaves this view mounted
  // under Conversations, and re-selecting / clearing here would undo
  // SessionInspector's selectSession(id) (SessionDetail) and
  // "← Back to conversation" (selectSession(null)).
  useEffect(() => {
    if (projectView !== "sessions") return;
    if (selectedSessionId) {
      // Valid if in project list OR same-project deep-link (inspector / openSession).
      if (
        resolveSessionForView(
          selectedSessionId,
          projectId,
          projectSessions,
          sessionsByConversation,
          conversationProjectById,
        )
      ) {
        return;
      }
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
    projectId,
    projectSessions,
    sessionsByConversation,
    conversationProjectById,
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

  // Degraded quiet poll of project sessions when live push is off.
  // Live path relies on manager events; no interval while livePush is healthy.
  useEffect(() => {
    if (source !== "daemon") return;
    if (livePush) return;
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
        <SessionListPane
          groups={groups}
          projectSessionCount={projectSessions.length}
          phase={phase}
          error={listStatus?.error}
          selectedSessionId={selectedSessionId}
          collapsedConvIds={collapsedConvIds}
          onToggleConversation={toggleConversation}
          onSelectSession={selectSession}
          onRetry={() => void loadProjectSessions(projectId)}
          onCollapseList={toggleSessionsList}
        />
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
  const resumeInterruptedSession = useWorkspaceStore(
    (s) => s.resumeInterruptedSession,
  );
  // Prefer L4 SessionEntity — select the entity by **reference** only.
  // Never `return { ...session, ...entity }` inside the selector: a new object
  // every getSnapshot trips useSyncExternalStore into max update depth.
  const sessionEntity = useWorkspaceStore((s) => s.sessionsById[sessionId]);
  const liveSession = useMemo((): ProjectSession => {
    if (sessionEntity) {
      return {
        ...session,
        id: sessionEntity.sessionId,
        conversationId:
          sessionEntity.conversationId || session.conversationId,
        conversationTitle:
          sessionEntity.conversationTitle ?? session.conversationTitle,
        agent: sessionEntity.agent as typeof session.agent,
        shortId: sessionEntity.shortId || session.shortId,
        status: sessionEntity.status,
        model: sessionEntity.model || session.model,
        parentId: sessionEntity.parentId ?? session.parentId,
        summary: sessionEntity.summary || session.summary,
        needsContinue: sessionEntity.needsContinue ?? session.needsContinue,
        firstTsMs: sessionEntity.firstTsMs ?? session.firstTsMs,
        lastTsMs: sessionEntity.lastTsMs ?? session.lastTsMs,
        messageCount: sessionEntity.messageCount ?? session.messageCount,
      };
    }
    return session;
  }, [sessionEntity, session]);
  const items = useWorkspaceStore(
    (s) => s.transcriptsBySession[sessionId] ?? EMPTY_TRANSCRIPT,
  );
  const status = useWorkspaceStore(
    (s) => s.transcriptStatusBySession[sessionId],
  );
  // Primitive fields only — never `?? newObject()` in a Zustand selector
  // (that re-allocates every getSnapshot and hits max update depth).
  const hasOlder = useWorkspaceStore(
    (s) => s.transcriptHistoryBySession[sessionId]?.hasOlder ?? false,
  );
  const loadingOlder = useWorkspaceStore(
    (s) => s.transcriptHistoryBySession[sessionId]?.loadingOlder ?? false,
  );
  const firstLoadedStartSeq = useWorkspaceStore(
    (s) =>
      s.transcriptHistoryBySession[sessionId]?.firstLoadedStartSeq ??
      EMPTY_TRANSCRIPT_HISTORY.firstLoadedStartSeq,
  );
  const source = useWorkspaceStore((s) => s.source);
  const livePush = useWorkspaceStore((s) => s.livePush);
  const [approving, setApproving] = useState<string | null>(null);
  const [summaryOpen, setSummaryOpen] = useState(true);

  /**
   * Identity-based restore for load-older prepends.
   * Height-delta restore races concurrent stream merges; we only restore when
   * `firstLoadedStartSeq` actually moves backward (older page applied).
   */
  const pendingOlderRestoreRef = useRef<{
    anchor: ItemScrollAnchor;
    firstLoadedStartSeqBefore: number;
  } | null>(null);
  /** Suspend stick-to-bottom pin while we own scrollTop after prepend. */
  const pinSuspendedRef = useRef(false);
  /** Prevent overlapping older fetches (state lag vs double effect fire). */
  const olderInFlightRef = useRef(false);
  const topSentinelRef = useRef<HTMLDivElement>(null);

  const contentKey = useMemo(() => followContentKey(items), [items]);
  const {
    scrollRef,
    contentRef,
    following,
    showJumpToLatest,
    followingRef,
    jumpToLatest,
    markProgrammatic,
    cancelScheduledPin,
  } = useStickToBottom({
    contentKey,
    resetKey: sessionId,
    pinSuspendedRef,
  });

  const summary = useMemo(
    () => summarizeSessionFromTranscript(items),
    [items],
  );

  // Cursor only while session is live *and* the timeline tail is still an
  // open text/reasoning bubble. Walking back past tools left a stuck █ on
  // finished narration (OpenCode task/subagent turns; same for other agents).
  const liveStreaming =
    session.status === "running" || session.status === "needs_approval";
  const streamingTailId = useMemo(() => streamingTailItemId(items), [items]);

  const loadOlder = useCallback(async () => {
    if (source !== "daemon") return;
    // Read live flags from store so this callback stays stable across stream ticks.
    const hist =
      useWorkspaceStore.getState().transcriptHistoryBySession[sessionId] ??
      EMPTY_TRANSCRIPT_HISTORY;
    if (!hist.hasOlder || hist.loadingOlder || olderInFlightRef.current) return;
    olderInFlightRef.current = true;

    // Only capture a viewport anchor when the user is in manual-scroll mode.
    // While following, stick-to-bottom owns the viewport (autofill / pin).
    const el = scrollRef.current;
    const content = contentRef.current;
    const shouldRestore = !followingRef.current;
    const currentItems =
      useWorkspaceStore.getState().transcriptsBySession[sessionId] ?? [];
    const seqBefore = hist.firstLoadedStartSeq;
    if (shouldRestore && el && content && currentItems.length > 0) {
      const topId = currentItems[0]!.id;
      const itemEl = queryScrollItem(content, topId);
      const anchor = captureItemScrollAnchor(el, topId, itemEl);
      if (anchor) {
        pendingOlderRestoreRef.current = {
          anchor,
          firstLoadedStartSeqBefore: seqBefore,
        };
      } else {
        pendingOlderRestoreRef.current = null;
      }
    } else {
      pendingOlderRestoreRef.current = null;
    }

    try {
      await loadTranscript(sessionId, {
        older: true,
        quiet: true,
        tailWindow: TRANSCRIPT_PAGE_EVENTS,
        approvalStatusPolicy: "sync",
      });
    } catch {
      pendingOlderRestoreRef.current = null;
    } finally {
      olderInFlightRef.current = false;
    }
  }, [source, loadTranscript, sessionId, scrollRef, contentRef, followingRef]);

  // After an older page lands (`firstLoadedStartSeq` decreased), restore the
  // anchored row. Ignore intermediate `items` updates (stream) while waiting.
  useLayoutEffect(() => {
    const pending = pendingOlderRestoreRef.current;
    if (!pending) return;
    // Older page not applied yet — keep pending across stream merges.
    if (firstLoadedStartSeq >= pending.firstLoadedStartSeqBefore) return;

    // User re-followed while the fetch was in flight: drop restore, pin owns it.
    if (followingRef.current) {
      pendingOlderRestoreRef.current = null;
      return;
    }

    const el = scrollRef.current;
    const content = contentRef.current;
    if (!el || !content) {
      pendingOlderRestoreRef.current = null;
      return;
    }

    const itemEl = queryScrollItem(content, pending.anchor.itemId);
    cancelScheduledPin();
    pinSuspendedRef.current = true;
    markProgrammatic(120);
    restoreItemScrollAnchor(el, itemEl, pending.anchor);
    pendingOlderRestoreRef.current = null;
    // Release pin suspend after layout settles so live follow can resume later.
    requestAnimationFrame(() => {
      pinSuspendedRef.current = false;
    });
    // Depend on firstLoadedStartSeq (older page applied) — not every stream
    // tick of `items`. DOM for the anchored id is updated in the same commit.
  }, [
    firstLoadedStartSeq,
    scrollRef,
    contentRef,
    followingRef,
    markProgrammatic,
    cancelScheduledPin,
  ]);

  // Init / reopen: first open loads the tail; when cache exists only append
  // new events (quiet) so switching tabs does not wipe older pages or flash
  // the loading skeleton.
  useEffect(() => {
    if (source !== "daemon") return;
    olderInFlightRef.current = false;
    pendingOlderRestoreRef.current = null;
    pinSuspendedRef.current = false;
    const cached =
      useWorkspaceStore.getState().transcriptsBySession[sessionId] ?? [];
    const hasCache = cached.length > 0;
    void loadTranscript(sessionId, {
      tailWindow: TRANSCRIPT_PAGE_EVENTS,
      quiet: hasCache,
      append: hasCache,
      approvalStatusPolicy: "sync",
    });
  }, [sessionId, source, loadTranscript]);

  // Desktop restart mid-turn leaves sessions as suspended + needs_continue.
  // Conversation open already auto-continues; opening a session transcript
  // must do the same so Sessions-tab users are not stuck on Paused.
  useEffect(() => {
    if (source !== "daemon") return;
    if (!liveSession.needsContinue) return;
    void resumeInterruptedSession(sessionId);
  }, [
    sessionId,
    source,
    liveSession.needsContinue,
    resumeInterruptedSession,
  ]);

  // Silent backfill only while stick-to-bottom is active (opening a session /
  // sparse tail). Never autofill while the user is reading older history —
  // that used to race with manual scroll and cause viewport thrash.
  // loadOlder skips identity restore while following, so pin is not fought.
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

  // Prefetch older via a top sentinel (not scrollTop threshold on every frame).
  // Fires only when the sentinel enters the scrollport in manual-scroll mode.
  useEffect(() => {
    const root = scrollRef.current;
    const sentinel = topSentinelRef.current;
    if (!root || !sentinel || typeof IntersectionObserver === "undefined") {
      return;
    }

    const io = new IntersectionObserver(
      (entries) => {
        const hit = entries.some((e) => e.isIntersecting);
        if (!hit) return;
        if (followingRef.current) return;
        void loadOlder();
      },
      { root, rootMargin: "120px 0px 0px 0px", threshold: 0 },
    );
    io.observe(sentinel);
    return () => io.disconnect();
  }, [sessionId, loadOlder, scrollRef, followingRef, hasOlder]);

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

  const onUserAction = useCallback(
    async (item: TranscriptItem, action: UserAction) => {
      if (!item.requestId) return;
      setApproving(item.requestId);
      try {
        await handleUserAction(session.id, item, action, {
          resolveApproval,
          respondOpencodePermission,
          respondOpencodeQuestion,
        });
        if (
          action.type === "decision" &&
          (item.kind === "approval" || item.kind === "question")
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
    },
    [
      session.id,
      resolveApproval,
      respondOpencodePermission,
      respondOpencodeQuestion,
    ],
  );

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
            className="scrollbar-thin min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-y-none px-4 py-4 sm:px-5"
            style={{ flex: "1 1 0%" }}
          >
            <div ref={contentRef} className="mx-auto max-w-3xl space-y-2.5 pb-8">
              {/* Sentinel for IntersectionObserver load-older (manual scroll). */}
              {hasOlder ? (
                <div
                  ref={topSentinelRef}
                  className="h-px w-full shrink-0"
                  aria-hidden
                />
              ) : null}
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
                  <div key={item.id} data-scroll-id={item.id}>
                    <TranscriptItemView
                      item={item}
                      streaming={itemShowsStreamingCursor(item, {
                        sessionLive: liveStreaming,
                        streamingTailId,
                      })}
                      approving={approving === item.requestId}
                      onUserAction={
                        item.requestId &&
                        (item.kind === "approval" || item.kind === "question")
                          ? onUserAction
                          : undefined
                      }
                    />
                  </div>
                ))
              )}
            </div>
          </div>

          {showJumpToLatest ? (
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
  sessionId: string,
  item: TranscriptItem,
  action: UserAction,
  apis: {
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
    await apis.respondOpencodePermission(sessionId, requestId, token);
    return;
  }

  if (method === "opencode/question") {
    if (action.type === "cancel") {
      await apis.respondOpencodeQuestion(sessionId, requestId, [[]]);
      return;
    }
    if (action.type === "answers") {
      await apis.respondOpencodeQuestion(sessionId, requestId, action.answers);
      return;
    }
    if (action.type === "decision") {
      await apis.respondOpencodeQuestion(sessionId, requestId, [
        [action.decision],
      ]);
    }
    return;
  }

  if (method === "x.ai/ask_user_question") {
    if (action.type === "cancel") {
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "cancelled",
      });
      return;
    }
    if (action.type === "answers") {
      const map: Record<string, string[]> = {};
      action.answers.forEach((a, i) => {
        if (a.length) map[String(i)] = a;
      });
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "accepted",
        answers: map,
      });
      return;
    }
    if (action.type === "decision") {
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "accepted",
        answers: { "0": [action.decision] },
      });
    }
    return;
  }

  // Plan / ACP permission / generic approval.
  if (action.type === "decision") {
    await apis.resolveApproval(sessionId, requestId, action.decision);
  } else if (action.type === "cancel") {
    await apis.resolveApproval(sessionId, requestId, "deny");
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
 * Memoized so stream/store ticks only re-render the active row + changed props.
 */
const TranscriptItemView = memo(function TranscriptItemView({
  item,
  streaming,
  onUserAction,
  approving,
}: {
  item: TranscriptItem;
  streaming?: boolean;
  onUserAction?: (
    item: TranscriptItem,
    action: UserAction,
  ) => void | Promise<void>;
  approving?: boolean;
}) {
  const [open, setOpen] = useState(Boolean(streaming));
  const [planOpen, setPlanOpen] = useState(false);

  const runAction = useCallback(
    (action: UserAction) => onUserAction?.(item, action),
    [onUserAction, item],
  );

  // Stream start re-opens thinking (TUI default expand while streaming).
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  if (item.kind === "approval" || item.kind === "question") {
    const isPlan = item.approvalMethod === "x.ai/exit_plan_mode";
    const isQuestion = item.kind === "question";
    // No requestId → already answered (history demote / local resolve). Do not
    // re-show interactive plan/permission chrome for a finished reverse-request.
    if (!item.requestId) {
      return (
        <div className="text-[12px] text-ink-muted">
          {item.title ? `${item.title} · ` : null}
          {item.text}
        </div>
      );
    }
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
                        void runAction({
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
                      void runAction({ type: "cancel" });
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
          onUserAction={runAction}
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

  // OpenCode / Codex subagent card (TUI SubagentCall parity).
  if (item.kind === "subagent") {
    const running = /\bRunning\b/i.test(item.text) || /\brunning\b/.test(item.text);
    const failed = /\bfailed\b/i.test(item.text) || /\binterrupted\b/i.test(item.text);
    const desc = (item.detail ?? "").trim();
    return (
      <div className="text-[12.5px] leading-snug">
        <div className="flex w-full max-w-full items-baseline gap-1.5">
          <span className="inline-block w-3 shrink-0" />
          <span
            className={cn(
              "shrink-0 font-medium",
              failed ? "text-rose-700" : "text-ink-secondary",
            )}
          >
            {item.text.split(/\s+/)[0] ?? (running ? "Running" : "Ran")}
          </span>
          <span
            className={cn(
              "min-w-0 truncate font-mono text-[12px]",
              failed ? "text-rose-800/90" : "text-ink",
            )}
            title={item.text}
          >
            {item.text.replace(/^(Running|Ran)\s+/i, "")}
          </span>
          {running ? (
            <span className="shrink-0 text-ink-muted">…</span>
          ) : null}
        </div>
        {desc ? (
          <p className="mt-0.5 pl-4 text-[12px] text-ink-muted line-clamp-2">
            {desc}
          </p>
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
});
