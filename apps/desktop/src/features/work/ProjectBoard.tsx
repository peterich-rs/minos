import { useMemo } from "react";
import { MessageSquare } from "lucide-react";
import {
  boardColumns,
  type Conversation,
  type ConversationBoardColumn,
} from "@/shared/lib/mock-data";
import { PriorityTag, ProgressTag } from "@/shared/ui/Tag";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";

/** Board uses conversation.progress as the single source of truth (no local overrides). */
export function ProjectBoard({ projectId }: { projectId: string }) {
  const selectConversation = useUiStore((s) => s.selectConversation);
  const moveConversationToBoardColumn = useWorkspaceStore(
    (s) => s.moveConversationToBoardColumn,
  );
  const conversations = useWorkspaceStore((s) => s.conversations);

  // Board columns are derived from progress + live session aggregates.
  const items = useMemo(
    () => conversations.filter((c) => c.projectId === projectId),
    [conversations, projectId],
  );

  return (
    <div className="scrollbar-thin flex min-h-0 flex-1 gap-3 overflow-x-auto bg-surface p-4">
      {boardColumns.map((col) => {
        const cards = items.filter((c) => c.boardColumn === col.id);
        return (
          <div
            key={col.id}
            className="flex w-[260px] shrink-0 flex-col rounded-2xl border border-ink/5 bg-surface-muted/30"
          >
            <div className="px-2 pt-2">
              <div
                className={cn(
                  "flex items-center justify-between rounded-xl px-3 py-2",
                  col.headerBg,
                )}
              >
                <div
                  className={cn("text-sm font-semibold", col.headerText)}
                >
                  {col.label}
                </div>
                <span
                  className={cn(
                    "rounded-md bg-surface-raised/70 px-1.5 py-0.5 text-2xs font-semibold tabular-nums",
                    col.headerText,
                  )}
                >
                  {cards.length}
                </span>
              </div>
            </div>
            <div className="scrollbar-thin flex flex-1 flex-col gap-2 overflow-y-auto px-2 py-2.5">
              {cards.map((card) => (
                <BoardCard
                  key={card.id}
                  card={card}
                  onOpen={() => selectConversation(card.id)}
                  onMove={(column) => {
                    void moveConversationToBoardColumn(card.id, column);
                  }}
                />
              ))}
              {cards.length === 0 ? (
                <div className="rounded-xl border border-dashed border-ink/10 bg-surface/60 px-3 py-6 text-center text-xs text-ink-muted">
                  No conversations
                </div>
              ) : null}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function BoardCard({
  card,
  onOpen,
  onMove,
}: {
  card: Conversation;
  onOpen: () => void;
  onMove: (column: ConversationBoardColumn) => void;
}) {
  return (
    <div className="rounded-xl border border-ink/5 bg-surface p-3 shadow-sm">
      <button type="button" onClick={onOpen} className="w-full text-left">
        <div className="flex items-start gap-2">
          <MessageSquare className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ink-muted" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold text-ink">{card.title}</div>
            <p className="mt-1 line-clamp-2 text-2xs leading-snug text-ink-muted">
              {card.preview}
            </p>
          </div>
        </div>
        <div className="mt-2.5 flex flex-wrap items-center gap-1">
          {card.priority ? (
            <PriorityTag priority={card.priority} size="sm" />
          ) : null}
          {card.progress ? (
            <ProgressTag progress={card.progress} size="sm" />
          ) : null}
          <span className="ml-auto text-3xs text-ink-muted">
            {card.updatedAt}
          </span>
        </div>
      </button>
      <div className="mt-2 flex flex-wrap gap-1 border-t border-ink/5 pt-2">
        {boardColumns
          .filter((c) => c.id !== card.boardColumn)
          .map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => onMove(c.id)}
              className="rounded-md px-1.5 py-0.5 text-3xs font-medium text-ink-muted hover:bg-surface-muted hover:text-ink"
            >
              → {c.label}
            </button>
          ))}
      </div>
    </div>
  );
}
