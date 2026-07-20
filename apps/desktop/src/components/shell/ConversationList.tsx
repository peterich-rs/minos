import { useEffect, useMemo } from "react";
import { MessageSquare, PanelLeftClose } from "lucide-react";
import type { Conversation } from "@/lib/mock-data";
import { sortByAttentionThenTime } from "@/lib/list-sort";
import { PriorityTag, ProgressTag } from "@/components/Tag";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/lib/utils";

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
  const selectConversation = useUiStore((s) => s.selectConversation);
  const toggleConversationList = useUiStore((s) => s.toggleConversationList);
  const conversations = useWorkspaceStore((s) => s.conversations);
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const loadConversations = useWorkspaceStore((s) => s.loadConversations);
  const listStatus = useWorkspaceStore(
    (s) => s.conversationsStatusByProject[projectId],
  );

  // Declarative init: load when project mounts / boot epoch advances.
  useEffect(() => {
    if (!projectId || source !== "daemon") return;
    void loadConversations(projectId);
  }, [projectId, source, loadConversations, bootEpoch]);

  const items = useMemo(() => {
    const sorted = [...conversations].filter(
      (c) => c.projectId === projectId,
    );
    sorted.sort((a, b) => {
      const aAttn = (a.unread ?? 0) + (a.approvalCount ?? 0);
      const bAttn = (b.unread ?? 0) + (b.approvalCount ?? 0);
      const aMs = a.updatedAtMs ?? 0;
      const bMs = b.updatedAtMs ?? 0;
      return sortByAttentionThenTime(
        { hasUnread: aAttn > 0, lastAttentionMs: aMs, updatedAtMs: aMs },
        { hasUnread: bAttn > 0, lastAttentionMs: bMs, updatedAtMs: bMs },
      );
    });
    return sorted;
  }, [conversations, projectId]);

  const phase = listStatus?.phase ?? "idle";

  return (
    <section
      className={cn(
        "flex h-full min-h-0 flex-col overflow-hidden border-r border-ink/5 bg-surface",
        fill
          ? "w-full min-w-0"
          : "w-[min(280px,34vw)] min-w-[220px] max-w-[340px] shrink-0",
      )}
    >
      <div className="flex shrink-0 items-center justify-between border-b border-ink/5 px-3 py-2.5">
        <div className="min-w-0 pl-1">
          <div className="text-[13px] font-semibold text-ink">Conversations</div>
          <div className="text-[11px] text-ink-muted">
            {phase === "loading" && items.length === 0
              ? "Loading…"
              : phase === "idle" && items.length === 0
                ? "Loading…"
                : `${items.length} in project`}
          </div>
        </div>
        <button
          type="button"
          title="Collapse conversation list"
          onClick={toggleConversationList}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition-colors hover:bg-surface-hover hover:text-ink"
        >
          <PanelLeftClose className="h-4 w-4" strokeWidth={1.8} />
        </button>
      </div>
      <div className="scrollbar-thin min-h-0 flex-1 space-y-0.5 overflow-y-auto p-2">
        {phase === "error" && items.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-2 py-8 text-center">
            <p className="text-[12px] text-rose-600">
              {listStatus?.error ?? "Failed to load conversations"}
            </p>
            <button
              type="button"
              onClick={() => void loadConversations(projectId)}
              className="rounded-lg bg-ink px-3 py-1.5 text-[11px] font-semibold text-white"
            >
              Retry
            </button>
          </div>
        ) : null}
        {items.map((item) => (
          <ConversationRow
            key={item.id}
            item={item}
            selected={item.id === conversationId}
            onSelect={() => selectConversation(item.id)}
          />
        ))}
        {phase === "ready" && items.length === 0 ? (
          <p className="px-2 py-6 text-center text-[12px] text-ink-muted">
            No conversations in this project.
          </p>
        ) : null}
        {(phase === "loading" || phase === "idle") && items.length === 0 ? (
          <p className="px-2 py-6 text-center text-[12px] text-ink-muted">
            Loading conversations…
          </p>
        ) : null}
      </div>
    </section>
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

function ConversationRow({
  item,
  selected,
  onSelect,
}: {
  item: Conversation;
  selected: boolean;
  onSelect: () => void;
}) {
  const hasTags = Boolean(item.priority || item.progress);
  // Attention badge = unread messages + pending approvals/suspended (not total message count).
  const attention =
    (item.unread ?? 0) + (item.approvalCount > 0 ? item.approvalCount : 0);
  const hasCounts = attention > 0;

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
      <MessageSquare className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ink-muted" />
      <div className="min-w-0 flex-1">
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-2">
          <span
            className="truncate text-[13px] font-semibold leading-snug text-ink"
            title={item.title}
          >
            {item.title}
          </span>
          <span className="shrink-0 text-[11px] tabular-nums text-ink-muted">
            {item.updatedAt}
          </span>
        </div>

        <p
          className="mt-0.5 line-clamp-2 text-[12px] leading-snug text-ink-muted"
          title={item.preview}
        >
          {item.preview}
        </p>

        {(hasTags || hasCounts) && (
          <div className="mt-1.5 flex flex-wrap items-center gap-1">
            {item.priority ? (
              <PriorityTag priority={item.priority} size="sm" />
            ) : null}
            {item.progress ? (
              <ProgressTag progress={item.progress} size="sm" />
            ) : null}
            {hasCounts ? (
              <span className="ml-auto rounded-full bg-rose-500 px-1.5 py-0.5 text-[10px] font-semibold text-white">
                {attention}
              </span>
            ) : null}
          </div>
        )}
      </div>
    </button>
  );
}
