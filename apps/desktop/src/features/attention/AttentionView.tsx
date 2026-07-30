import { useEffect, useMemo, useState } from "react";
import {
  ShieldAlert,
  XCircle,
  PauseCircle,
  MessageSquare,
} from "lucide-react";
import { agentMeta } from "@/shared/lib/mock-data";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";
import {
  buildAttentionInbox,
  countAttentionInboxByCategory,
  filterAttentionInbox,
  type AttentionInboxCategory,
  type AttentionInboxFilter,
  type AttentionInboxItem,
} from "./lib/attention-inbox";

const FILTERS: { id: AttentionInboxFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "approval", label: "Approvals" },
  { id: "unread", label: "Unread" },
  { id: "failed", label: "Failed" },
  { id: "suspended", label: "Paused" },
];

export function AttentionView() {
  const selectConversation = useUiStore((s) => s.selectConversation);
  const selectProject = useUiStore((s) => s.selectProject);
  const selectSession = useUiStore((s) => s.selectSession);
  const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
  const setPrimaryNav = useUiStore((s) => s.setPrimaryNav);
  const source = useWorkspaceStore((s) => s.source);
  const sessions = useWorkspaceStore((s) => s.attentionSessions);
  const status = useWorkspaceStore((s) => s.attentionStatus);
  const loadAttentionSessions = useWorkspaceStore(
    (s) => s.loadAttentionSessions,
  );
  const projects = useWorkspaceStore((s) => s.projects);
  const conversations = useWorkspaceStore((s) => s.conversations);

  const [filter, setFilter] = useState<AttentionInboxFilter>("all");

  useEffect(() => {
    if (source !== "daemon") return;
    void loadAttentionSessions();
  }, [source, loadAttentionSessions]);

  const inbox = useMemo(
    () =>
      buildAttentionInbox({
        conversations,
        projects,
        sessions,
      }),
    [conversations, projects, sessions],
  );
  const counts = useMemo(() => countAttentionInboxByCategory(inbox), [inbox]);
  const items = useMemo(
    () => filterAttentionInbox(inbox, filter),
    [inbox, filter],
  );

  const phase = status.phase;
  const sessionHydrating = phase === "loading" && sessions.length === 0;

  const openConversation = (item: AttentionInboxItem) => {
    if (item.projectId) selectProject(item.projectId);
    selectConversation(item.conversationId);
    setPrimaryNav("work");
  };

  const openSession = (item: AttentionInboxItem) => {
    if (!item.sessionId) {
      openConversation(item);
      return;
    }
    if (item.projectId) selectProject(item.projectId);
    selectSession(item.sessionId);
    openSessionTranscript(item.sessionId, item.conversationId);
    setPrimaryNav("work");
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="border-b border-ink/5 px-6 py-5">
        <h1 className="text-xl font-semibold tracking-tight text-ink">
          Attention
        </h1>
        <p className="mt-1 text-sm text-ink-muted">
          Your inbox: unread messages, approvals, and sessions that need a
          follow-up.
        </p>
        <div className="mt-4 flex flex-wrap gap-1.5">
          {FILTERS.map((f) => {
            const count = counts[f.id];
            const active = filter === f.id;
            if (f.id !== "all" && count === 0) return null;
            return (
              <button
                key={f.id}
                type="button"
                onClick={() => setFilter(f.id)}
                className={cn(
                  "rounded-full px-3 py-1 text-xs font-medium transition-colors",
                  active
                    ? "bg-ink text-surface"
                    : "bg-surface-muted text-ink-secondary hover:bg-surface-hover",
                )}
              >
                {f.label}
                {count > 0 ? (
                  <span
                    className={cn(
                      "ml-1.5 tabular-nums",
                      active ? "text-surface/80" : "text-ink-muted",
                    )}
                  >
                    {count}
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>
      </header>
      <div className="scrollbar-thin flex-1 space-y-2 overflow-y-auto p-4">
        {sessionHydrating && items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            Scanning sessions…
          </p>
        ) : null}
        {phase === "error" && sessions.length === 0 && items.length === 0 ? (
          <div className="flex flex-col items-center gap-3 py-12 text-center">
            <p className="text-sm text-rose-600">
              {status.error ?? "Failed to load attention queue"}
            </p>
            <button
              type="button"
              onClick={() => void loadAttentionSessions()}
              className="rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface"
            >
              Retry
            </button>
          </div>
        ) : null}
        {!sessionHydrating && items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            {filter === "all"
              ? "Nothing needs attention right now."
              : "Nothing in this filter."}
          </p>
        ) : null}
        {items.map((item) => (
          <AttentionCard
            key={item.id}
            item={item}
            onOpenConversation={() => openConversation(item)}
            onOpenSession={() => openSession(item)}
          />
        ))}
      </div>
    </div>
  );
}

function AttentionCard({
  item,
  onOpenConversation,
  onOpenSession,
}: {
  item: AttentionInboxItem;
  onOpenConversation: () => void;
  onOpenSession: () => void;
}) {
  const meta = item.agent
    ? agentMeta[item.agent as keyof typeof agentMeta]
    : undefined;
  const isApproval = item.category === "approval";
  const isUnread = item.category === "unread";
  const Icon = iconForCategory(item.category);

  return (
    <div className="rounded-2xl border border-ink/5 bg-surface-raised p-4 shadow-sm">
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "flex h-9 w-9 items-center justify-center rounded-xl",
            iconSurface(item.category),
          )}
        >
          <Icon className="h-4 w-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-ink">{item.title}</span>
            {item.agent && item.shortId ? (
              <span
                className={cn(
                  "rounded-md px-1.5 py-0.5 text-2xs font-medium",
                  meta?.color ?? "bg-ink/10 text-ink-secondary",
                )}
              >
                {meta?.label ?? item.agent} #{item.shortId}
              </span>
            ) : null}
            {isUnread && item.unreadCount ? (
              <span className="rounded-md bg-accent/15 px-1.5 py-0.5 text-2xs font-semibold text-accent">
                {item.unreadCount}
              </span>
            ) : null}
          </div>
          {item.preview ? (
            <p className="mt-1 line-clamp-2 text-sm text-ink-secondary">
              {item.preview}
            </p>
          ) : null}
          <p className="mt-1 text-xs text-ink-muted">
            {item.projectName} / {item.conversationTitle}
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={onOpenConversation}
              className="rounded-lg border border-ink/10 bg-surface-raised px-3 py-1.5 text-xs font-medium text-ink hover:bg-surface-muted"
            >
              Open conversation
            </button>
            {!isUnread ? (
              <button
                type="button"
                onClick={onOpenSession}
                className="rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface hover:opacity-90"
              >
                {isApproval ? "Review / approve" : "Open transcript"}
              </button>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

function iconForCategory(category: AttentionInboxCategory) {
  switch (category) {
    case "approval":
      return ShieldAlert;
    case "failed":
      return XCircle;
    case "suspended":
      return PauseCircle;
    case "unread":
      return MessageSquare;
  }
}

function iconSurface(category: AttentionInboxCategory): string {
  switch (category) {
    case "approval":
      return "bg-rose-100 text-rose-700";
    case "failed":
      return "bg-red-100 text-red-700";
    case "suspended":
      return "bg-status-suspended/15 text-status-suspended";
    case "unread":
      return "bg-accent/15 text-accent";
  }
}
