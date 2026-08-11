import { useEffect, useRef, useState } from "react";
import { GitBranch, Layers } from "lucide-react";
import type { Conversation } from "@/shared/domain/collaboration";
import {
  MetaChip,
  PriorityPlaceholder,
  PriorityTag,
  ProgressTag,
} from "@/shared/ui/Tag";
import { useWorkspaceStore } from "@/store/workspace-store";
import { shortWorktree } from "./lib/format";
import { toast } from "@/shared/lib/toast";

export function TimelineHeader({
  conversationId,
  conversation,
  sessionCount,
}: {
  conversationId: string;
  conversation: Conversation;
  sessionCount: number;
}) {
  const updateConversationTitle = useWorkspaceStore(
    (s) => s.updateConversationTitle,
  );
  const cycleConversationPriority = useWorkspaceStore(
    (s) => s.cycleConversationPriority,
  );
  const cycleConversationProgress = useWorkspaceStore(
    (s) => s.cycleConversationProgress,
  );

  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setEditingTitle(false);
    setTitleDraft("");
  }, [conversationId]);

  useEffect(() => {
    if (editingTitle) {
      titleInputRef.current?.focus();
      titleInputRef.current?.select();
    }
  }, [editingTitle]);

  const beginEditTitle = () => {
    setTitleDraft(conversation.title);
    setEditingTitle(true);
  };

  const commitTitle = async () => {
    if (!conversationId || !editingTitle) return;
    const next = titleDraft.trim();
    setEditingTitle(false);
    if (!next || next === conversation.title) return;
    try {
      await updateConversationTitle(conversationId, next);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error("Failed to rename conversation", msg);
    }
  };

  const cancelTitle = () => {
    setEditingTitle(false);
    setTitleDraft("");
  };

  return (
    <header className="flex shrink-0 items-center justify-between gap-3 border-b border-ink/6 bg-surface/90 px-4 py-3.5 backdrop-blur-sm sm:px-5">
      <div className="min-w-0 flex-1">
        {editingTitle ? (
          <input
            ref={titleInputRef}
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            onBlur={() => {
              void commitTitle();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void commitTitle();
              } else if (e.key === "Escape") {
                e.preventDefault();
                cancelTitle();
              }
            }}
            className="w-full min-w-0 rounded-md border border-ink/10 bg-surface-muted px-2 py-0.5 text-base font-semibold tracking-tight text-ink outline-none ring-primary/30 focus:ring-2"
            aria-label="Conversation title"
          />
        ) : (
          <h2
            className="cursor-text truncate rounded-md text-base font-semibold tracking-tight text-ink hover:bg-ink/[0.04]"
            title={`${conversation.title} — double-click to rename`}
            onDoubleClick={beginEditTitle}
          >
            {conversation.title}
          </h2>
        )}
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
          {conversation.branch ? (
            <MetaChip>
              <GitBranch className="h-3 w-3 shrink-0 text-ink-muted" />
              <span className="truncate">{conversation.branch}</span>
              {conversation.gitDirty ? (
                <span
                  role="img"
                  className="ml-0.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500"
                  title={
                    conversation.gitHead
                      ? `Uncommitted changes @ ${conversation.gitHead}`
                      : "Uncommitted changes"
                  }
                  aria-label="dirty working tree"
                />
              ) : null}
            </MetaChip>
          ) : null}
          {conversation.worktree ? (
            <MetaChip>
              <Layers className="h-3 w-3 shrink-0 text-ink-muted" />
              <span className="truncate" title={conversation.worktree}>
                {shortWorktree(conversation.worktree)}
              </span>
            </MetaChip>
          ) : null}
          {sessionCount > 0 ? (
            <span className="text-2xs text-ink-muted">
              {sessionCount} agent session
              {sessionCount === 1 ? "" : "s"}
            </span>
          ) : null}
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
        {conversation.priority ? (
          <PriorityTag
            priority={conversation.priority}
            onClick={() => {
              if (conversationId) {
                void cycleConversationPriority(conversationId);
              }
            }}
            title="Click to cycle priority"
          />
        ) : (
          <PriorityPlaceholder
            onClick={() => {
              if (conversationId) {
                void cycleConversationPriority(conversationId);
              }
            }}
          />
        )}
        <ProgressTag
          progress={conversation.progress ?? "todo"}
          onClick={() => {
            if (conversationId) {
              void cycleConversationProgress(conversationId);
            }
          }}
          title="Click to cycle progress"
        />
      </div>
    </header>
  );
}
