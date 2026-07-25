import { memo, useMemo } from "react";
import {
  ChevronDown,
  ChevronRight,
  Loader2,
  MessageSquare,
  PanelLeftClose,
} from "lucide-react";
import { useStableArrayShallow } from "@/shared/hooks/useStableReference";
import { agentMeta, statusMeta } from "@/shared/lib/mock-data";
import { Avatar } from "@/shared/ui/Avatar";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import { cn } from "@/shared/lib/utils";
import type { ProjectSession } from "@/store/workspace-store";
import {
  flattenSessionListRows,
  sessionIsExecuting,
  type ConversationSessionGroup,
  type VirtualSessionListRow,
} from "@/shared/lib/session-list-group";

/**
 * Sessions tab left rail: conversation folders → agent runs.
 * Virtualized for long project histories; tree is flattened for the list.
 */
export function SessionListPane({
  groups,
  projectSessionCount,
  phase,
  error,
  selectedSessionId,
  collapsedConvIds,
  onToggleConversation,
  onSelectSession,
  onRetry,
  onCollapseList,
}: {
  groups: ConversationSessionGroup[];
  projectSessionCount: number;
  phase: string;
  error?: string | null;
  selectedSessionId: string | null;
  collapsedConvIds: ReadonlySet<string>;
  onToggleConversation: (conversationId: string) => void;
  onSelectSession: (id: string) => void;
  onRetry: () => void;
  onCollapseList: () => void;
}) {
  const rows = useStableArrayShallow(
    useMemo(
      () => flattenSessionListRows(groups, collapsedConvIds),
      [groups, collapsedConvIds],
    ),
  );
  const conversationCount = groups.length;
  const liveTotal = groups.reduce((n, g) => n + g.runningCount, 0);

  return (
    <aside className="flex w-[min(300px,36vw)] min-w-[240px] max-w-[360px] shrink-0 flex-col overflow-hidden border-r border-ink/5 bg-surface">
      <div className="flex shrink-0 items-center justify-between border-b border-ink/5 px-3 py-2.5">
        <div className="min-w-0 pl-1">
          <div className="text-sm font-semibold text-ink">Sessions</div>
          <div className="text-2xs text-ink-muted">
            {phase === "loading" && projectSessionCount === 0
              ? "Loading…"
              : conversationCount === 0
                ? "No sessions"
                : `${conversationCount} conversation${conversationCount === 1 ? "" : "s"} · ${projectSessionCount} session${projectSessionCount === 1 ? "" : "s"}${liveTotal > 0 ? ` · ${liveTotal} live` : ""}`}
          </div>
        </div>
        <button
          type="button"
          title="Collapse"
          onClick={onCollapseList}
          className="flex h-8 w-8 items-center justify-center rounded-lg text-ink-muted hover:bg-surface-hover"
        >
          <PanelLeftClose className="h-4 w-4" />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        {phase === "error" && projectSessionCount === 0 ? (
          <div className="flex flex-col items-center gap-2 px-2 py-8 text-center">
            <p className="text-xs text-rose-600">
              {error ?? "Failed to load sessions"}
            </p>
            <button
              type="button"
              onClick={onRetry}
              className="rounded-lg bg-ink px-3 py-1.5 text-2xs font-semibold text-surface"
            >
              Retry
            </button>
          </div>
        ) : null}
        {phase === "loading" && projectSessionCount === 0 ? (
          <p className="px-2 py-8 text-center text-xs text-ink-muted">
            Loading sessions…
          </p>
        ) : null}
        {phase === "ready" && projectSessionCount === 0 ? (
          <p className="px-2 py-8 text-center text-xs text-ink-muted">
            No agent sessions yet. Use @agent in a conversation.
          </p>
        ) : null}
        {rows.length > 0 ? (
          <VirtualizedList
            className="min-h-0 flex-1 px-2 py-2"
            items={rows}
            getItemKey={(row) => row.key}
            estimateSize={64}
            overscan={8}
            renderItem={(row) => (
              <div className="pb-0.5">
                <SessionListRow
                  row={row}
                  selectedSessionId={selectedSessionId}
                  onToggleConversation={onToggleConversation}
                  onSelectSession={onSelectSession}
                />
              </div>
            )}
          />
        ) : null}
      </div>
    </aside>
  );
}

const SessionListRow = memo(function SessionListRow({
  row,
  selectedSessionId,
  onToggleConversation,
  onSelectSession,
}: {
  row: VirtualSessionListRow;
  selectedSessionId: string | null;
  onToggleConversation: (conversationId: string) => void;
  onSelectSession: (id: string) => void;
}) {
  if (row.type === "folder") {
    return (
      <ConversationSessionFolderHeader
        group={row.group}
        collapsed={row.collapsed}
        selectedSessionId={selectedSessionId}
        onToggleConversation={onToggleConversation}
      />
    );
  }
  if (row.type === "empty-roots") {
    return (
      <p className="px-3 py-2 text-2xs text-ink-muted">No top-level sessions</p>
    );
  }
  return (
    <SessionTreeRow
      session={row.session}
      depth={row.depth}
      selectedId={selectedSessionId}
      onSelect={onSelectSession}
    />
  );
});

const ConversationSessionFolderHeader = memo(
  function ConversationSessionFolderHeader({
    group,
    collapsed,
    selectedSessionId,
    onToggleConversation,
  }: {
    group: ConversationSessionGroup;
    collapsed: boolean;
    selectedSessionId: string | null;
    /** Stable parent callback — row calls with conversationId (no per-row closure). */
    onToggleConversation: (conversationId: string) => void;
  }) {
  const hasSelected = group.sessions.some((s) => s.id === selectedSessionId);

  return (
    <button
      type="button"
      onClick={() => onToggleConversation(group.conversationId)}
      className={cn(
        "flex w-full items-center gap-1.5 rounded-xl px-2 py-2 text-left transition-colors",
        "hover:bg-surface-hover",
        hasSelected && collapsed ? "bg-surface-muted/60" : null,
        hasSelected && !collapsed ? "bg-surface-muted/40" : null,
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
        className="min-w-0 flex-1 truncate text-xs font-semibold text-ink"
        title={group.title}
      >
        {group.title}
      </span>
      {group.runningCount > 0 ? (
        <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-status-running/20 px-1.5 py-0.5 text-3xs font-medium text-status-running">
          <Loader2 className="h-3 w-3 animate-spin" />
          {group.runningCount}
        </span>
      ) : null}
      {group.attentionCount > 0 && group.runningCount === 0 ? (
        <span className="shrink-0 rounded-full bg-status-failed/20 px-1.5 py-0.5 text-3xs font-medium text-status-failed">
          {group.attentionCount}
        </span>
      ) : null}
      <span className="shrink-0 text-3xs tabular-nums text-ink-muted">
        {group.sessions.length}
      </span>
    </button>
  );
},
);

const SessionTreeRow = memo(function SessionTreeRow({
  session,
  depth,
  selectedId,
  onSelect,
}: {
  session: ProjectSession;
  depth: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const selected = selectedId === session.id;
  const executing = sessionIsExecuting(session.status);
  const status = statusMeta[session.status] ?? statusMeta.idle;

  return (
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
        <div className="min-w-0">
          <span className="truncate text-xs font-semibold text-ink">
            {meta?.label ?? session.agent}{" "}
            <span className="font-mono text-3xs font-normal text-ink-muted">
              #{session.shortId}
            </span>
          </span>
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
          <span
            className={cn(
              "inline-flex max-w-full items-center gap-1 truncate rounded-full px-1.5 py-0.5 text-3xs font-medium",
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
            <span className="truncate text-3xs text-ink-muted">subagent</span>
          ) : null}
        </div>
        {session.summary ? (
          <p
            className="mt-0.5 line-clamp-1 text-2xs leading-snug text-ink-muted"
            title={session.summary}
          >
            {session.summary}
          </p>
        ) : null}
      </div>
    </button>
  );
});
