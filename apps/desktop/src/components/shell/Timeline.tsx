import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ArrowDown,
  AtSign,
  Bold,
  GitBranch,
  Layers,
  Paperclip,
  Send,
  Wrench,
} from "lucide-react";
import { agentMeta, type TimelineMessage } from "@/lib/mock-data";
import {
  KNOWN_AGENTS,
  mentionQueryAtCursor,
  type KnownAgent,
} from "@/lib/agent-route";
import { Avatar } from "@/components/Avatar";
import { MarkdownText } from "@/components/MarkdownText";
import {
  MetaChip,
  PriorityPlaceholder,
  PriorityTag,
  ProgressTag,
} from "@/components/Tag";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/lib/utils";
import { toast } from "@/lib/toast";
import { followContentKey } from "@/lib/stick-to-bottom";
import { useStickToBottom } from "@/lib/use-stick-to-bottom";

/** Empty shell when no conversation is selected (outer nav owns selection). */
export function TimelineEmpty() {
  return (
    <div className="flex flex-1 items-center justify-center bg-surface text-[13px] text-ink-muted">
      Select a conversation or create one to start.
    </div>
  );
}

/**
 * Declarative conversation timeline: parent passes `conversationId` (and
 * ideally `key={conversationId}`). This view owns detail init for that id;
 * render only subscribes to workspace state.
 */
export function Timeline({ conversationId }: { conversationId: string }) {
  const draft = useUiStore(
    (s) => s.draftByConversationId[conversationId] ?? "",
  );
  const setDraftGlobal = useUiStore((s) => s.setDraft);
  const setDraft = (value: string) => setDraftGlobal(conversationId, value);
  const conversations = useWorkspaceStore((s) => s.conversations);
  const messagesByConversation = useWorkspaceStore(
    (s) => s.messagesByConversation,
  );
  const detailStatus = useWorkspaceStore(
    (s) => s.detailStatusByConversation[conversationId],
  );
  const loadConversationDetail = useWorkspaceStore(
    (s) => s.loadConversationDetail,
  );
  const sendMessage = useWorkspaceStore((s) => s.sendMessage);
  const updateConversationTitle = useWorkspaceStore(
    (s) => s.updateConversationTitle,
  );
  const cycleConversationPriority = useWorkspaceStore(
    (s) => s.cycleConversationPriority,
  );
  const cycleConversationProgress = useWorkspaceStore(
    (s) => s.cycleConversationProgress,
  );
  const source = useWorkspaceStore((s) => s.source);
  const clis = useWorkspaceStore((s) => s.clis);
  const sessionsByConversation = useWorkspaceStore(
    (s) => s.sessionsByConversation,
  );
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const titleInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const conversation = conversations.find((c) => c.id === conversationId);
  const messages = messagesByConversation[conversationId] ?? [];
  const sessions = sessionsByConversation[conversationId] ?? [];
  const phase = detailStatus?.phase ?? "idle";
  const detailError = detailStatus?.error;
  const hasCachedMessages = conversationId in messagesByConversation;

  const contentKey = useMemo(
    () =>
      followContentKey(
        messages.map((m) => ({
          id: m.id,
          kind: m.kind,
          text: m.body,
        })),
      ),
    [messages],
  );
  const { scrollRef, contentRef, following, jumpToLatest } = useStickToBottom({
    contentKey,
    resetKey: conversationId,
  });

  const mention = mentionQueryAtCursor(draft, cursor);
  const mentionOptions = useMemo(() => {
    if (!mention) return [];
    const q = mention.query.toLowerCase();
    const fromCli = clis
      .filter((c) => c.agent.includes(q))
      .map((c) => ({
        id: c.agent,
        label: `@${c.agent}`,
        hint: c.installed ? c.status : "not installed",
        insert: `@${c.agent} `,
        disabled: !c.installed,
      }));
    // existing sessions in this conversation
    const fromSessions = sessions
      .filter((s) => !s.parentId)
      .filter(
        (s) =>
          s.agent.includes(q) ||
          s.shortId.toLowerCase().includes(q) ||
          `@${s.agent}#${s.shortId}`.includes(q),
      )
      .map((s) => ({
        id: s.id,
        label: `@${s.agent}#${s.shortId}`,
        hint: s.status,
        insert: `@${s.agent}#${s.shortId} `,
        disabled: false,
      }));
    // always offer known agents if cli list empty
    const base =
      fromCli.length > 0
        ? fromCli
        : KNOWN_AGENTS.filter((a) => a.includes(q)).map((a) => ({
            id: a,
            label: `@${a}`,
            hint: "agent",
            insert: `@${a} `,
            disabled: false,
          }));
    return [...base, ...fromSessions].slice(0, 12);
  }, [mention, clis, sessions]);

  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const livePush = useWorkspaceStore((s) => s.livePush);

  // Init: load detail whenever this view is mounted for conversationId
  // (re-run after boot wipe via bootEpoch).
  useEffect(() => {
    if (source !== "daemon") return;
    void loadConversationDetail(conversationId);
  }, [conversationId, source, loadConversationDetail, bootEpoch]);

  // Fallback quiet poll ONLY when live push is unavailable (browser mock or
  // subscribe failure). With daemon://* pumps, status/messages arrive via events.
  const hasLiveSession = sessions.some(
    (s) => s.status === "running" || s.status === "needs_approval",
  );
  const expectHistoryEmpty =
    (conversation?.messageCount ?? 0) > 0 && messages.length === 0;
  useEffect(() => {
    if (source !== "daemon" || livePush) return;
    const needPoll =
      hasLiveSession || phase === "error" || expectHistoryEmpty;
    if (!needPoll) return;
    const id = window.setInterval(() => {
      void loadConversationDetail(conversationId, { quiet: true });
    }, 2500);
    return () => window.clearInterval(id);
  }, [
    conversationId,
    source,
    livePush,
    hasLiveSession,
    expectHistoryEmpty,
    phase,
    loadConversationDetail,
  ]);

  useEffect(() => {
    setMentionIndex(0);
  }, [mention?.query, mentionOptions.length]);

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

  if (!conversation) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 bg-surface px-6 text-center text-[13px] text-ink-muted">
        <p>Conversation not found in the current project list.</p>
        {source === "daemon" ? (
          <button
            type="button"
            onClick={() => void loadConversationDetail(conversationId)}
            className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white"
          >
            Retry load
          </button>
        ) : null}
      </div>
    );
  }

  const applyMention = (insert: string) => {
    if (!mention) return;
    const next =
      draft.slice(0, mention.start) + insert + draft.slice(cursor);
    setDraft(next);
    const nextCursor = mention.start + insert.length;
    setCursor(nextCursor);
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(nextCursor, nextCursor);
      }
    });
  };

  const onSend = async () => {
    const text = draft.trim();
    if (!text || !conversationId) return;
    setSending(true);
    setSendError(null);
    try {
      await sendMessage(conversationId, text);
      setDraft("");
      setCursor(0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSendError(msg);
      toast.error("Failed to send message", msg);
    } finally {
      setSending(false);
    }
  };

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
      setSendError(msg);
      toast.error("Failed to rename conversation", msg);
    }
  };

  const cancelTitle = () => {
    setEditingTitle(false);
    setTitleDraft("");
  };

  return (
    <section className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-surface">
      <header className="flex shrink-0 items-center justify-between gap-3 border-b border-ink/5 px-4 py-3 sm:px-5">
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
              className="w-full min-w-0 rounded-md border border-ink/10 bg-surface-muted px-2 py-0.5 text-[15px] font-semibold tracking-tight text-ink outline-none ring-accent/30 focus:ring-2"
              aria-label="Conversation title"
            />
          ) : (
            <h2
              className="cursor-text truncate rounded-md text-[15px] font-semibold tracking-tight text-ink hover:bg-surface-muted/80"
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
            {sessions.length > 0 ? (
              <span className="text-[11px] text-ink-muted">
                {sessions.length} agent session
                {sessions.length === 1 ? "" : "s"}
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

      <div
        ref={scrollRef}
        className="scrollbar-thin min-h-0 flex-1 overflow-y-auto overscroll-y-contain px-5 py-5"
        style={{ flex: "1 1 0%" }}
      >
        <div ref={contentRef} className="space-y-4">
          {phase === "loading" && !hasCachedMessages ? (
            <div className="py-12 text-center text-[13px] text-ink-muted">
              Loading messages…
            </div>
          ) : phase === "error" && !hasCachedMessages ? (
            <div className="flex flex-col items-center gap-3 py-12 text-center">
              <p className="text-[13px] text-rose-600">
                {detailError || "Failed to load messages"}
              </p>
              <button
                type="button"
                onClick={() => void loadConversationDetail(conversationId)}
                className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white hover:opacity-90"
              >
                Retry
              </button>
            </div>
          ) : messages.length === 0 ? (
            <div className="py-12 text-center text-[13px] text-ink-muted">
              No messages yet. Type{" "}
              <kbd className="rounded bg-surface-muted px-1.5 py-0.5 font-mono text-[12px]">
                @grok
              </kbd>{" "}
              or{" "}
              <kbd className="rounded bg-surface-muted px-1.5 py-0.5 font-mono text-[12px]">
                @codex
              </kbd>{" "}
              to start an agent.
            </div>
          ) : (
            messages.map((message) => (
              <TimelineRow
                key={message.id}
                message={message}
                replyParent={
                  message.replyToMessageId
                    ? messages.find((m) => m.id === message.replyToMessageId)
                    : undefined
                }
              />
            ))
          )}
        </div>
      </div>

      {!following ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-[7.5rem] z-10 flex justify-center sm:bottom-36">
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

      <div className="relative shrink-0 border-t border-ink/5 px-5 py-4">
        {mention && mentionOptions.length > 0 ? (
          <div className="absolute bottom-full left-5 right-5 mb-2 max-h-52 overflow-y-auto rounded-xl border border-ink/10 bg-surface py-1 shadow-lg">
            <div className="px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide text-ink-muted">
              Agents
            </div>
            {mentionOptions.map((opt, i) => (
              <button
                key={opt.id}
                type="button"
                disabled={opt.disabled}
                onMouseDown={(e) => {
                  e.preventDefault();
                  if (!opt.disabled) applyMention(opt.insert);
                }}
                className={cn(
                  "flex w-full items-center justify-between px-3 py-2 text-left text-[13px]",
                  i === mentionIndex
                    ? "bg-surface-muted"
                    : "hover:bg-surface-hover",
                  opt.disabled && "opacity-40",
                )}
              >
                <span className="font-medium text-ink">{opt.label}</span>
                <span className="text-[11px] text-ink-muted">{opt.hint}</span>
              </button>
            ))}
          </div>
        ) : null}

        <div className="rounded-2xl border border-ink/10 bg-surface-muted/40 shadow-sm">
          <textarea
            ref={textareaRef}
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              setCursor(e.target.selectionStart);
            }}
            onSelect={(e) =>
              setCursor((e.target as HTMLTextAreaElement).selectionStart)
            }
            onClick={(e) =>
              setCursor((e.target as HTMLTextAreaElement).selectionStart)
            }
            onKeyDown={(e) => {
              if (mention && mentionOptions.length > 0) {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setMentionIndex((i) =>
                    Math.min(i + 1, mentionOptions.length - 1),
                  );
                  return;
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setMentionIndex((i) => Math.max(i - 1, 0));
                  return;
                }
                if (e.key === "Enter" || e.key === "Tab") {
                  const opt = mentionOptions[mentionIndex];
                  if (opt && !opt.disabled) {
                    e.preventDefault();
                    applyMention(opt.insert);
                    return;
                  }
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setCursor(mention.start);
                  return;
                }
              }
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                void onSend();
              }
            }}
            rows={3}
            placeholder="Message… type @ to mention an agent (e.g. @grok hello)"
            className="w-full resize-none rounded-t-2xl bg-transparent px-4 pt-3 text-[13.5px] text-ink outline-none placeholder:text-ink-muted"
          />
          <div className="flex items-center justify-between px-3 pb-3">
            <div className="flex items-center gap-0.5 text-ink-muted">
              <ToolBtn
                onClick={() => {
                  const el = textareaRef.current;
                  const pos = el?.selectionStart ?? draft.length;
                  const next =
                    draft.slice(0, pos) + "@" + draft.slice(pos);
                  setDraft(next);
                  const c = pos + 1;
                  setCursor(c);
                  requestAnimationFrame(() => {
                    el?.focus();
                    el?.setSelectionRange(c, c);
                  });
                }}
              >
                <AtSign className="h-3.5 w-3.5" />
              </ToolBtn>
              <ToolBtn>
                <Bold className="h-3.5 w-3.5" />
              </ToolBtn>
              <ToolBtn>
                <Paperclip className="h-3.5 w-3.5" />
              </ToolBtn>
            </div>
            <button
              type="button"
              disabled={sending || !draft.trim()}
              onClick={() => void onSend()}
              className="inline-flex items-center gap-1.5 rounded-xl bg-ink px-3.5 py-2 text-[12.5px] font-semibold text-white hover:opacity-90 disabled:opacity-40"
            >
              {sending ? "Sending…" : "Send"}
              <Send className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        <p className="mt-2 px-1 text-[11px] text-ink-muted">
          {source === "daemon"
            ? "Connected · @agent starts a session · ⌘/Ctrl+Enter to send"
            : "Mock mode"}
          {phase === "loading" && hasCachedMessages ? " · refreshing…" : ""}
        </p>
        {sendError || (phase === "error" && detailError) ? (
          <p className="mt-1 px-1 text-[12px] text-rose-600">
            {sendError || detailError}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function shortWorktree(path: string): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

function replyPreviewBody(body: string, maxChars = 120): string {
  const collapsed = body
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join(" ");
  if (collapsed.length <= maxChars) return collapsed;
  return `${collapsed.slice(0, maxChars - 1)}…`;
}

function replyAuthorLabel(parent: TimelineMessage): string {
  if (parent.role === "user") return "You";
  if (parent.role === "system") return "System";
  const agentKey = parent.agent as KnownAgent | undefined;
  if (agentKey && agentMeta[agentKey]) return agentMeta[agentKey].label;
  return parent.agent ?? "Agent";
}

function TimelineRow({
  message,
  replyParent,
}: {
  message: TimelineMessage;
  replyParent?: TimelineMessage;
}) {
  if (message.role === "system") {
    return (
      <div className="mx-auto max-w-md animate-message-in rounded-xl bg-surface-muted px-3 py-2 text-center text-[12px] text-ink-muted motion-reduce:animate-none">
        {message.body}
      </div>
    );
  }

  const isUser = message.role === "user";
  const agentKey = message.agent as KnownAgent | undefined;
  const agent = agentKey && agentMeta[agentKey] ? agentMeta[agentKey] : null;

  // Conversation timeline must not invent "approval" cards from free text.
  // Real approvals (permission / plan / opencode question) live on the session
  // transcript with a requestId and wired Allow/Deny — same as TUI.
  // Live run status / open-session CTAs stay in the right-hand Session inspector
  // (TUI parity: do not pollute the chat timeline with session chrome).

  if (message.kind === "tool_summary") {
    return (
      <div className="flex w-full animate-message-in items-center gap-2 rounded-xl border border-ink/5 bg-surface-muted/80 px-3 py-2 text-left text-[12px] text-ink-secondary motion-reduce:animate-none">
        <Wrench className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 truncate">{message.body}</span>
        <span className="shrink-0 text-ink-muted">{message.time}</span>
      </div>
    );
  }

  return (
    <div
      className={cn(
        "flex animate-message-in gap-2.5 motion-reduce:animate-none",
        isUser ? "justify-end" : "justify-start",
      )}
    >
      {!isUser ? (
        <Avatar name={agent?.label ?? "Agent"} tone={agent?.tone ?? "slate"} />
      ) : null}
      <div className={cn("max-w-[78%] space-y-1", isUser && "items-end")}>
        {!isUser ? (
          <div className="flex items-center gap-1.5 px-1 text-[12px]">
            <span className="font-medium text-ink">
              {agent?.label ?? message.agent ?? "Agent"}
            </span>
            {message.pending ? (
              <span className="text-[11px] text-ink-muted">sending…</span>
            ) : null}
          </div>
        ) : null}
        {message.replyToMessageId ? (
          <div
            className={cn(
              "rounded-lg border-l-2 px-2.5 py-1.5 text-[11.5px] leading-snug",
              isUser
                ? "border-white/40 bg-black/15 text-white/85"
                : "border-ink/20 bg-surface-muted/80 text-ink-secondary",
            )}
          >
            <div className={cn("font-medium", isUser ? "text-white" : "text-ink")}>
              ↳ {replyParent ? replyAuthorLabel(replyParent) : "Reply"}
            </div>
            <div className="mt-0.5 line-clamp-2 opacity-90">
              {replyParent
                ? replyPreviewBody(replyParent.body)
                : `(reply unavailable · ${message.replyToMessageId})`}
            </div>
          </div>
        ) : null}
        <div
          className={cn(
            "rounded-2xl px-3.5 py-2.5 text-[13.5px] leading-relaxed shadow-sm",
            isUser
              ? "rounded-br-md bg-bubble-out text-white"
              : "rounded-bl-md border border-ink/5 bg-surface-muted/60 text-ink",
            message.pending && "opacity-70",
          )}
        >
          <MarkdownText
            text={message.body}
            tone={isUser ? "onDark" : "default"}
            className="text-[13.5px]"
          />
        </div>
        <div
          className={cn(
            "px-1 text-[11px] text-ink-muted",
            isUser && "text-right",
          )}
        >
          {message.pending ? "sending…" : message.time}
        </div>
      </div>
    </div>
  );
}

function ToolBtn({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-7 w-7 items-center justify-center rounded-md hover:bg-surface-hover hover:text-ink"
    >
      {children}
    </button>
  );
}
