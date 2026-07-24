import {
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
  Loader2,
  MessageSquare,
  PanelRightClose,
  PanelRightOpen,
} from "lucide-react";
import { agentMeta } from "@/shared/lib/mock-data";
import { Avatar } from "@/shared/ui/Avatar";
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
  EMPTY_TRANSCRIPT_HISTORY,
  TRANSCRIPT_AUTOFILL_SLACK_PX,
  TRANSCRIPT_PAGE_EVENTS,
} from "@/shared/lib/transcript-history";
import {
  itemShowsStreamingCursor,
  streamingTailItemId,
} from "@/shared/lib/transcript-streaming";
import { summarizeSessionFromTranscript } from "@/shared/lib/session-summary";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { handleUserAction, type UserAction } from "../lib/user-action";
import { SessionSummaryPanel } from "./SessionSummaryPanel";
import { TranscriptItemView } from "./TranscriptItemView";

/**
 * Stable empty snapshot for Zustand selectors.
 * `?? []` inside a selector returns a new array every getSnapshot call, which
 * trips React useSyncExternalStore into "Maximum update depth exceeded".
 */
const EMPTY_TRANSCRIPT: TranscriptItem[] = [];

export function TranscriptPane({
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
