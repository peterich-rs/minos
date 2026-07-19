import { useEffect, useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowLeft,
  Bot,
  ChevronDown,
  ChevronRight,
  MessageSquare,
  PanelLeftClose,
  ShieldAlert,
} from "lucide-react";
import { agentMeta } from "@/lib/mock-data";
import { Avatar } from "@/components/Avatar";
import { MarkdownText } from "@/components/MarkdownText";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { formatLocalClock, formatRelative } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { TranscriptItem } from "@/lib/daemon";
import { followContentKey } from "@/lib/stick-to-bottom";
import { useStickToBottom } from "@/lib/use-stick-to-bottom";
import {
  buildToolHeader,
  collapsedThinkingSummary,
} from "@/lib/tool-present";

/**
 * Project sessions tab — parent passes projectId (and key).
 * List load is owned here; transcript load is owned by TranscriptPane.
 */
export function SessionsView({ projectId }: { projectId: string }) {
  const selectedSessionId = useUiStore((s) => s.selectedSessionId);
  const selectSession = useUiStore((s) => s.selectSession);
  const openConversation = useUiStore((s) => s.openConversation);
  const listCollapsed = useUiStore((s) => s.sessionsListCollapsed);
  const toggleSessionsList = useUiStore((s) => s.toggleSessionsList);

  const projectSessions = useWorkspaceStore(
    (s) => s.projectSessionsByProject[projectId] ?? s.projectSessions,
  );
  const listStatus = useWorkspaceStore(
    (s) => s.projectSessionsStatusByProject[projectId],
  );
  const loadProjectSessions = useWorkspaceStore((s) => s.loadProjectSessions);
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const livePush = useWorkspaceStore((s) => s.livePush);

  // Init: load project sessions for this projectId (re-run after boot wipe).
  useEffect(() => {
    if (source !== "daemon") return;
    void loadProjectSessions(projectId);
  }, [projectId, source, loadProjectSessions, bootEpoch]);

  // Auto-select first session when list arrives.
  useEffect(() => {
    if (selectedSessionId) {
      if (projectSessions.some((s) => s.id === selectedSessionId)) return;
    }
    if (projectSessions.length > 0) selectSession(projectSessions[0]!.id);
  }, [projectSessions, selectedSessionId, selectSession]);

  // Fallback list poll only without live push (manager events own live status).
  useEffect(() => {
    if (source !== "daemon" || livePush) return;
    const live = projectSessions.some(
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
    projectSessions,
    listStatus?.phase,
    loadProjectSessions,
  ]);

  const selected = projectSessions.find((s) => s.id === selectedSessionId);
  const phase = listStatus?.phase ?? "idle";

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
        <aside className="flex w-[min(280px,34vw)] min-w-[220px] max-w-[340px] shrink-0 flex-col overflow-hidden border-r border-ink/5 bg-surface">
          <div className="flex shrink-0 items-center justify-between border-b border-ink/5 px-3 py-2.5">
            <div className="min-w-0 pl-1">
              <div className="text-[13px] font-semibold text-ink">Sessions</div>
              <div className="text-[11px] text-ink-muted">
                {phase === "loading" && projectSessions.length === 0
                  ? "Loading…"
                  : `${projectSessions.length} in project`}
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

          <div className="scrollbar-thin min-h-0 flex-1 space-y-0.5 overflow-y-auto p-2">
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
            {projectSessions.map((session) => (
              <SessionRow
                key={session.id}
                session={session}
                selected={session.id === selectedSessionId}
                onSelect={() => selectSession(session.id)}
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

function SessionRow({
  session,
  selected,
  onSelect,
}: {
  session: ProjectSession;
  selected: boolean;
  onSelect: () => void;
}) {
  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const when = session.lastTsMs ? formatRelative(session.lastTsMs) : undefined;

  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex w-full gap-2.5 rounded-xl px-3 py-2.5 text-left transition-colors",
        selected
          ? "bg-surface-muted shadow-panel ring-1 ring-ink/5"
          : "hover:bg-surface-hover",
      )}
    >
      <Avatar
        name={meta?.label ?? session.agent}
        tone={meta?.tone ?? "slate"}
        size="sm"
      />
      <div className="min-w-0 flex-1">
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-2">
          <span className="truncate text-[13px] font-semibold text-ink">
            {meta?.label ?? session.agent}{" "}
            <span className="font-mono text-[11px] font-normal text-ink-muted">
              #{session.shortId}
            </span>
          </span>
          {when ? (
            <span className="text-[11px] tabular-nums text-ink-muted">
              {when}
            </span>
          ) : null}
        </div>
        <p
          className="mt-0.5 line-clamp-2 text-[12px] leading-snug text-ink-muted"
          title={session.summary}
        >
          {session.summary}
        </p>
        {session.conversationTitle ? (
          <p
            className="mt-1 truncate text-[10px] text-ink-muted"
            title={session.conversationTitle}
          >
            {session.conversationTitle}
          </p>
        ) : null}
      </div>
    </button>
  );
}

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
  const loadTranscript = useWorkspaceStore((s) => s.loadTranscript);
  const items = useWorkspaceStore(
    (s) => s.transcriptsByThread[sessionId] ?? [],
  );
  const status = useWorkspaceStore(
    (s) => s.transcriptStatusByThread[sessionId],
  );
  const source = useWorkspaceStore((s) => s.source);
  const livePush = useWorkspaceStore((s) => s.livePush);
  const [approving, setApproving] = useState<string | null>(null);

  const contentKey = useMemo(() => followContentKey(items), [items]);
  const { scrollRef, contentRef, following, jumpToLatest } = useStickToBottom({
    contentKey,
    resetKey: sessionId,
  });

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

  // Init: load transcript for this sessionId (view-owned).
  // sync policy: full tail open may clear needs_approval when no pending cards.
  useEffect(() => {
    if (source !== "daemon") return;
    void loadTranscript(sessionId, {
      tailWindow: 400,
      approvalStatusPolicy: "sync",
    });
  }, [sessionId, source, loadTranscript]);

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
      </header>

      <div
        ref={scrollRef}
        className="scrollbar-thin relative min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-5"
      >
        <div ref={contentRef} className="mx-auto max-w-3xl space-y-2.5">
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
                  void loadTranscript(sessionId, { tailWindow: 400 })
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
                onApprove={
                  item.kind === "approval" && item.requestId
                    ? async (decision) => {
                        setApproving(item.requestId!);
                        try {
                          await resolveApproval(
                            session.id,
                            item.requestId!,
                            decision,
                          );
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
    </section>
  );
}

function ApprovalModal({
  item,
  isPlan,
  approving,
  onClose,
  onApprove,
}: {
  item: TranscriptItem;
  isPlan: boolean;
  approving?: boolean;
  onClose: () => void;
  onApprove?: (decision: "approve" | "revise" | "abandon") => void | Promise<void>;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4 sm:p-8"
      role="dialog"
      aria-modal="true"
      aria-label={item.title ?? "Approval"}
      onClick={onClose}
    >
      <div
        className="flex max-h-[min(85vh,720px)] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-ink/10 bg-surface shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex shrink-0 items-start justify-between gap-3 border-b border-ink/5 px-5 py-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-[15px] font-semibold text-ink">
              <ShieldAlert className="h-4 w-4 shrink-0 text-rose-600" />
              {item.title ?? "Approval required"}
            </div>
            <p className="mt-1 text-[12.5px] text-ink-muted">{item.text}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-2 py-1 text-[12px] font-medium text-ink-muted hover:bg-surface-muted hover:text-ink"
          >
            Close
          </button>
        </header>
        <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {item.detail ? (
            <pre className="whitespace-pre-wrap font-mono text-[12.5px] leading-relaxed text-ink-secondary">
              {item.detail}
            </pre>
          ) : (
            <p className="text-[13px] text-ink-muted">No additional detail.</p>
          )}
        </div>
        {onApprove ? (
          <footer className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-ink/5 bg-surface-muted/40 px-5 py-3.5">
            <button
              type="button"
              disabled={approving}
              onClick={() => {
                void (async () => {
                  await onApprove("abandon");
                  onClose();
                })();
              }}
              className="rounded-lg border border-ink/10 bg-white px-3 py-2 text-[12px] font-medium text-ink-muted hover:bg-surface disabled:opacity-50"
            >
              {isPlan ? "Abandon" : "Deny"}
            </button>
            {isPlan ? (
              <button
                type="button"
                disabled={approving}
                onClick={() => {
                  void (async () => {
                    await onApprove("revise");
                    onClose();
                  })();
                }}
                className="rounded-lg border border-ink/10 bg-white px-3 py-2 text-[12px] font-medium text-ink hover:bg-surface disabled:opacity-50"
              >
                Request changes
              </button>
            ) : null}
            <button
              type="button"
              disabled={approving}
              onClick={() => {
                void (async () => {
                  await onApprove("approve");
                  onClose();
                })();
              }}
              className="rounded-lg bg-ink px-3.5 py-2 text-[12px] font-semibold text-white hover:bg-ink/90 disabled:opacity-50"
            >
              {isPlan ? "Approve plan" : "Allow"}
            </button>
          </footer>
        ) : null}
      </div>
    </div>
  );
}

/**
 * Grok / TUI AgentDetail-style transcript row (not messenger bubbles).
 */
function TranscriptItemView({
  item,
  streaming,
  onApprove,
  approving,
}: {
  item: TranscriptItem;
  streaming?: boolean;
  onApprove?: (decision: "approve" | "revise" | "abandon") => void | Promise<void>;
  approving?: boolean;
}) {
  const time = item.tsMs ? formatLocalClock(item.tsMs) : "";
  const [open, setOpen] = useState(Boolean(streaming));
  const [planOpen, setPlanOpen] = useState(false);

  // Stream start re-opens thinking (TUI default expand while streaming).
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  if (item.kind === "approval") {
    const isPlan = item.approvalMethod === "x.ai/exit_plan_mode";
    return (
      <>
        <div className="rounded-xl border border-rose-200/80 bg-rose-50/80 p-3">
          <div className="flex items-start gap-2.5">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-rose-600" />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-rose-900">
                {item.title ?? "Approval required"}
              </div>
              <p className="mt-1 text-[12.5px] leading-snug text-rose-900/80">
                {item.text}
              </p>
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
            </div>
          </div>
        </div>
        {planOpen ? (
          <ApprovalModal
            item={item}
            isPlan={isPlan}
            approving={approving}
            onClose={() => setPlanOpen(false)}
            onApprove={onApprove}
          />
        ) : null}
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
    return (
      <MarkdownText text={item.text} streaming={streaming} />
    );
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
    const expandable = Boolean(item.detail?.trim());
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
        {open && item.detail ? (
          <pre className="mt-1 max-h-72 overflow-auto rounded-lg border border-ink/5 bg-surface-muted/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-ink-secondary whitespace-pre-wrap">
            {item.detail}
          </pre>
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
    return (
      <div className="text-[12px] text-ink-muted">{item.text}</div>
    );
  }

  return (
    <div className="text-[11px] text-ink-muted">
      {item.title ?? item.kind}
      {item.text ? ` · ${item.text.slice(0, 120)}` : ""}
    </div>
  );
}
