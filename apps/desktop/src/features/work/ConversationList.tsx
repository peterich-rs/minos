import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, ListFilter, MessageSquare, PanelLeftClose } from "lucide-react";
import type { Conversation } from "@/shared/lib/mock-data";
import {
  matchesProgressFilter,
  progressFilterLabel,
  PROGRESS_FILTER_OPTIONS,
  type ConversationProgressFilter,
} from "@/shared/lib/conversation-meta";
import { useStableArrayShallow } from "@/shared/hooks/useStableReference";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { PriorityTag, ProgressTag } from "@/shared/ui/Tag";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import {
  WorkConversationRail,
  WorkConversationRow,
} from "@/shared/ui/WorkChrome";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";
import { formatListActivityTime } from "@/shared/lib/time";

/**
 * Project-scoped conversation list.
 * Parent passes `projectId` (and ideally `key={projectId}`). This view owns
 * list init for that id; render subscribes to workspace state.
 */
export function ConversationList({
  projectId,
  /** Fill a resizable panel instead of a fixed width rail. */
  fill = false,
}: {
  projectId: string;
  fill?: boolean;
}) {
  const conversationId = useUiStore((s) => s.conversationId);
  const projectView = useUiStore((s) => s.projectView);
  const selectConversation = useUiStore((s) => s.selectConversation);
  const toggleConversationList = useUiStore((s) => s.toggleConversationList);
  /** Keep-alive under Sessions/Board — only mount virtualizer while visible. */
  const listActive = projectView === "conversations";
  const conversations = useWorkspaceStore((s) => s.conversations);
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const loadConversations = useWorkspaceStore((s) => s.loadConversations);
  const listStatus = useWorkspaceStore(
    (s) => s.conversationsStatusByProject[projectId],
  );
  /** Default All: show every conversation in the project. */
  const [progressFilter, setProgressFilter] =
    useState<ConversationProgressFilter>("all");

  // Declarative init: load when project mounts / boot epoch advances.
  useEffect(() => {
    if (!projectId || source !== "daemon") return;
    void loadConversations(projectId);
  }, [projectId, source, loadConversations, bootEpoch]);

  const { items: itemsRaw, projectCount } = useMemo(() => {
    const inProject = conversations.filter((c) => c.projectId === projectId);
    const filtered = inProject.filter((c) =>
      matchesProgressFilter(c.progress, progressFilter),
    );
    filtered.sort((a, b) => {
      const aAttn = (a.unread ?? 0) + (a.approvalCount ?? 0);
      const bAttn = (b.unread ?? 0) + (b.approvalCount ?? 0);
      const aMs = a.updatedAtMs ?? 0;
      const bMs = b.updatedAtMs ?? 0;
      return sortByAttentionThenTime(
        { hasUnread: aAttn > 0, updatedAtMs: aMs },
        { hasUnread: bAttn > 0, updatedAtMs: bMs },
      );
    });
    return { items: filtered, projectCount: inProject.length };
  }, [conversations, projectId, progressFilter]);
  // Preserve row list identity when filter/sort yields the same Conversation refs.
  const items = useStableArrayShallow(itemsRaw);

  const handleSelectConversation = useCallback(
    (id: string) => {
      selectConversation(id);
    },
    [selectConversation],
  );

  const phase = listStatus?.phase ?? "idle";
  const filterLabel = progressFilterLabel(progressFilter);
  const isFiltered = progressFilter !== "all";
  const countLabel = (() => {
    if (
      (phase === "loading" || phase === "idle") &&
      projectCount === 0
    ) {
      return "Loading…";
    }
    if (!isFiltered) {
      return `${projectCount} in project`;
    }
    return `${items.length} of ${projectCount} · ${filterLabel}`;
  })();

  return (
    <WorkConversationRail
      fill={fill}
      subtitle={countLabel}
      actions={
        <div className="flex shrink-0 items-center gap-0.5">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                title={`Filter by status: ${filterLabel}`}
                aria-label={`Filter conversations by status, currently ${filterLabel}`}
                className={cn(
                  "flex h-8 max-w-[7.5rem] items-center gap-1 rounded-lg px-1.5 text-ink-muted transition-colors hover:bg-surface-hover hover:text-ink",
                  isFiltered && "bg-surface-muted text-ink",
                )}
              >
                <ListFilter className="h-3.5 w-3.5 shrink-0" strokeWidth={1.8} />
                <span className="truncate text-2xs font-medium">
                  {filterLabel}
                </span>
                <ChevronDown className="h-3 w-3 shrink-0 opacity-70" strokeWidth={2} />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-[10rem]">
              <DropdownMenuRadioGroup
                value={progressFilter}
                onValueChange={(value) =>
                  setProgressFilter(value as ConversationProgressFilter)
                }
              >
                {PROGRESS_FILTER_OPTIONS.map((opt) => (
                  <DropdownMenuRadioItem key={opt.value} value={opt.value}>
                    {opt.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
          <button
            type="button"
            title="Collapse conversation list"
            onClick={toggleConversationList}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition-colors hover:bg-surface-hover hover:text-ink"
          >
            <PanelLeftClose className="h-4 w-4" strokeWidth={1.8} />
          </button>
        </div>
      }
    >
      {phase === "error" && projectCount === 0 ? (
        <div className="flex flex-col items-center gap-2 px-2 py-8 text-center">
          <p className="text-xs text-rose-600">
            {listStatus?.error ?? "Failed to load conversations"}
          </p>
          <button
            type="button"
            onClick={() => void loadConversations(projectId)}
            className="rounded-lg bg-primary px-3 py-1.5 text-2xs font-semibold text-white shadow-sm"
          >
            Retry
          </button>
        </div>
      ) : null}
      {phase === "ready" && projectCount === 0 ? (
        <p className="px-2 py-6 text-center text-xs text-ink-muted">
          No conversations in this project.
        </p>
      ) : null}
      {phase === "ready" && projectCount > 0 && items.length === 0 ? (
        <div className="flex flex-col items-center gap-2 px-2 py-6 text-center">
          <p className="text-xs text-ink-muted">
            No {filterLabel.toLowerCase()} conversations.
          </p>
          <button
            type="button"
            onClick={() => setProgressFilter("all")}
            className="rounded-lg bg-surface-muted px-3 py-1.5 text-2xs font-semibold text-ink ring-1 ring-ink/10 transition-colors hover:bg-surface-hover"
          >
            Show all
          </button>
        </div>
      ) : null}
      {(phase === "loading" || phase === "idle") && projectCount === 0 ? (
        <p className="px-2 py-6 text-center text-xs text-ink-muted">
          Loading conversations…
        </p>
      ) : null}
      {listActive && items.length > 0 ? (
        <VirtualizedList
          key="conversation-rail-active"
          className="min-h-0 flex-1"
          items={items}
          getItemKey={(item) => item.id}
          estimateSize={88}
          overscan={6}
          renderItem={(item) => (
            <div className="pb-0.5">
              <ConversationRow
                item={item}
                selected={item.id === conversationId}
                onSelect={handleSelectConversation}
              />
            </div>
          )}
        />
      ) : null}
    </WorkConversationRail>
  );
}

export function ConversationListRail() {
  const toggleConversationList = useUiStore((s) => s.toggleConversationList);

  return (
    <div className="flex w-10 shrink-0 flex-col items-center border-r border-ink/5 bg-surface pt-2.5">
      <button
        type="button"
        title="Expand conversation list"
        onClick={toggleConversationList}
        className="flex h-8 w-8 items-center justify-center rounded-lg text-ink-muted transition-colors hover:bg-surface-hover hover:text-ink"
      >
        <MessageSquare className="h-4 w-4" strokeWidth={1.8} />
      </button>
    </div>
  );
}

/**
 * Memoized row: `item` must keep reference identity when content is unchanged
 * (workspace list patches should reuse Conversation objects). `onSelect` is the
 * stable `(id) => void` from the parent — never an inline per-row closure.
 */
const ConversationRow = memo(function ConversationRow({
  item,
  selected,
  onSelect,
}: {
  item: Conversation;
  selected: boolean;
  onSelect: (id: string) => void;
}) {
  const hasTags = Boolean(item.priority || item.progress);
  // Attention badge = unread messages + pending approvals/suspended (not total message count).
  const attention =
    (item.unread ?? 0) + (item.approvalCount > 0 ? item.approvalCount : 0);
  const hasCounts = attention > 0;

  return (
    <WorkConversationRow
      title={item.title}
      preview={item.preview}
      selected={selected}
      onSelect={() => onSelect(item.id)}
      titleTrailing={
        <span className="shrink-0 text-2xs tabular-nums text-ink-muted">
          {formatListActivityTime(item.updatedAtMs)}
        </span>
      }
      meta={
        hasTags || hasCounts ? (
          <>
            {item.priority ? (
              <PriorityTag priority={item.priority} size="sm" />
            ) : null}
            {item.progress ? (
              <ProgressTag progress={item.progress} size="sm" />
            ) : null}
            {hasCounts ? (
              <span className="ml-auto rounded-full bg-rose-500 px-1.5 py-0.5 text-3xs font-semibold text-white">
                {attention}
              </span>
            ) : null}
          </>
        ) : undefined
      }
    />
  );
});
