import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { AtSign, Bold, Paperclip, Send, X } from "lucide-react";
import {
  buildAgentMentionOptions,
  mentionQueryAtCursor,
  type MentionProfile,
} from "@/shared/lib/agent-route";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { cn } from "@/shared/lib/utils";
import { toast } from "@/shared/lib/toast";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { EmojiPicker } from "./EmojiPicker";
import { replyAuthorLabel, replyPreviewBody } from "./lib/format";

const EMPTY_SESSIONS: ProjectSession[] = [];
const EMPTY_MESSAGES: TimelineMessage[] = [];
const EMPTY_PROFILES: MentionProfile[] = [];

export function Composer({ conversationId }: { conversationId: string }) {
  const draft = useUiStore(
    (s) => s.draftByConversationId[conversationId] ?? "",
  );
  const setDraftGlobal = useUiStore((s) => s.setDraft);
  const setDraft = (value: string) => setDraftGlobal(conversationId, value);
  const replyToMessageId = useUiStore(
    (s) => s.replyToMessageIdByConversation[conversationId] ?? null,
  );
  const clearReplyTo = useUiStore((s) => s.clearReplyTo);
  const messages = useWorkspaceStore(
    (s) => s.messagesByConversation[conversationId] ?? EMPTY_MESSAGES,
  );
  const replyParent = useMemo(
    () =>
      replyToMessageId
        ? messages.find((m) => m.id === replyToMessageId)
        : undefined,
    [messages, replyToMessageId],
  );

  const sendMessage = useWorkspaceStore((s) => s.sendMessage);
  const loadInspector = useWorkspaceStore((s) => s.loadInspector);
  const source = useWorkspaceStore((s) => s.source);
  const clis = useWorkspaceStore((s) => s.clis);
  const participatingAgents = useWorkspaceStore(
    (s) =>
      s.conversations.find((c) => c.id === conversationId)
        ?.participatingAgents ?? [],
  );
  const sessions = useWorkspaceStore(
    (s) => s.sessionsByConversation[conversationId] ?? EMPTY_SESSIONS,
  );
  const timelineStatus = useWorkspaceStore(
    (s) => s.timelineStatusByConversation[conversationId],
  );
  const hasCachedMessages = useWorkspaceStore(
    (s) => conversationId in s.messagesByConversation,
  );

  const phase = timelineStatus?.phase ?? "idle";
  const detailError = timelineStatus?.error;

  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionProfiles, setMentionProfiles] =
    useState<MentionProfile[]>(EMPTY_PROFILES);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const mention = mentionQueryAtCursor(draft, cursor);
  const mentionOptions = useMemo(() => {
    if (!mention) return [];
    // Membership-gated: only conversation roster agents + their profiles/sessions.
    return buildAgentMentionOptions({
      query: mention.query,
      clis,
      sessions,
      profiles: mentionProfiles,
      memberAgents: participatingAgents,
    });
  }, [mention, clis, sessions, mentionProfiles, participatingAgents]);

  // When @-mention UI opens and sessions are empty, ensure Inspector working set
  // so @agent#short options can list existing sessions without opening the rail.
  const mentionActive = mention != null;
  useEffect(() => {
    if (source !== "daemon" || !mentionActive) return;
    if (sessions.length > 0) return;
    const hasKey =
      conversationId in useWorkspaceStore.getState().sessionsByConversation;
    void loadInspector(conversationId, { quiet: hasKey });
  }, [mentionActive, sessions.length, conversationId, source, loadInspector]);

  // Load host agent profiles for @ProfileName options while the picker is open.
  useEffect(() => {
    if (source !== "daemon" || !mentionActive || !isTauriRuntime()) return;
    let cancelled = false;
    void (async () => {
      try {
        const res = await daemonApi.listAgentProfiles();
        if (cancelled) return;
        setMentionProfiles(
          (res.profiles ?? []).map((p) => ({
            id: p.id,
            name: p.name,
            runtimeAgent: p.runtime_agent,
          })),
        );
      } catch {
        if (!cancelled) setMentionProfiles(EMPTY_PROFILES);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [mentionActive, source]);

  useEffect(() => {
    setMentionIndex(0);
  }, [mention?.query, mentionOptions.length]);

  // Focus composer when user picks Reply on a message.
  useEffect(() => {
    if (!replyToMessageId) return;
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
    });
  }, [replyToMessageId]);

  const applyMention = (insert: string) => {
    if (!mention) return;
    const next = draft.slice(0, mention.start) + insert + draft.slice(cursor);
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

  const insertAtCursor = (text: string) => {
    const el = textareaRef.current;
    const pos = el?.selectionStart ?? draft.length;
    const next = draft.slice(0, pos) + text + draft.slice(pos);
    setDraft(next);
    const c = pos + text.length;
    setCursor(c);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(c, c);
    });
  };

  const onSend = async () => {
    const text = draft.trim();
    if (!text || !conversationId) return;
    // WeChat-style: empty the composer immediately. The message body is
    // already captured in `text`; the optimistic `sending` row (inserted by
    // sendMessage before any throwing step) carries it. On failure the row
    // becomes a failed bubble with a red `!`, so the draft is never refilled.
    const pendingReplyTo = replyToMessageId ?? undefined;
    setDraft("");
    setCursor(0);
    clearReplyTo(conversationId);
    setSending(true);
    setSendError(null);
    try {
      await sendMessage(conversationId, text, undefined, {
        replyToMessageId: pendingReplyTo,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSendError(msg);
      toast.error("Failed to send message", msg);
    } finally {
      setSending(false);
    }
  };

  return (
    // Composer stays outside the scrollport — always visible at the bottom.
    <div className="relative shrink-0 border-t border-ink/5 bg-surface px-5 py-4">
      {mention && mentionOptions.length > 0 ? (
        <div className="absolute bottom-full left-5 right-5 mb-2 max-h-52 overflow-y-auto rounded-xl border border-ink/10 bg-surface py-1 shadow-lg">
          <div className="px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-ink-muted">
            Agents & profiles
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
                "flex w-full items-center justify-between px-3 py-2 text-left text-sm",
                i === mentionIndex
                  ? "bg-surface-muted"
                  : "hover:bg-surface-hover",
                opt.disabled && "opacity-40",
              )}
            >
              <span className="font-medium text-ink">{opt.label}</span>
              <span className="text-2xs text-ink-muted">{opt.hint}</span>
            </button>
          ))}
        </div>
      ) : null}

      {replyToMessageId ? (
        <div className="mb-2 flex items-start gap-2 rounded-xl border border-ink/10 bg-surface-muted/50 px-3 py-2">
          <div className="min-w-0 flex-1">
            <div className="text-2xs font-semibold text-ink">
              Replying to{" "}
              {replyParent ? replyAuthorLabel(replyParent) : "message"}
            </div>
            <div className="mt-0.5 line-clamp-2 text-xs text-ink-secondary">
              {replyParent
                ? replyPreviewBody(replyParent.body)
                : `(message ${replyToMessageId})`}
            </div>
          </div>
          <button
            type="button"
            onClick={() => clearReplyTo(conversationId)}
            aria-label="Cancel reply"
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-surface-hover hover:text-ink"
          >
            <X className="h-3.5 w-3.5" />
          </button>
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
            if (e.key === "Escape" && replyToMessageId) {
              e.preventDefault();
              clearReplyTo(conversationId);
              return;
            }
            if (e.key === "Enter" && hasPrimaryShortcutModifier(e)) {
              e.preventDefault();
              void onSend();
            }
          }}
          rows={3}
          placeholder="Message… type @ to mention an agent (e.g. @grok hello)"
          className="w-full resize-none rounded-t-2xl bg-transparent px-4 pt-3 text-sm text-ink outline-none placeholder:text-ink-muted"
        />
        <div className="flex items-center justify-between px-3 pb-3">
          <div className="flex items-center gap-0.5 text-ink-muted">
            <ToolBtn
              onClick={() => {
                insertAtCursor("@");
              }}
            >
              <AtSign className="h-3.5 w-3.5" />
            </ToolBtn>
            <ToolBtn>
              <Bold className="h-3.5 w-3.5" />
            </ToolBtn>
            <EmojiPicker
              onSelect={(emoji) => insertAtCursor(emoji)}
              showQuickStrip={false}
              side="top"
              align="start"
              ariaLabel="Insert emoji"
            />
            <ToolBtn>
              <Paperclip className="h-3.5 w-3.5" />
            </ToolBtn>
          </div>
          <button
            type="button"
            disabled={sending || !draft.trim()}
            onClick={() => void onSend()}
            className="inline-flex items-center gap-1.5 rounded-xl bg-ink px-3.5 py-2 text-xs font-semibold text-surface hover:opacity-90 disabled:opacity-40"
          >
            {sending ? "Sending…" : "Send"}
            <Send className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      <p className="mt-2 px-1 text-2xs text-ink-muted">
        {source === "daemon"
          ? "Connected · @member agent · @agent#id continue · ⌘/Ctrl+Enter send"
          : "Mock mode"}
        {phase === "loading" && hasCachedMessages ? " · refreshing…" : ""}
      </p>
      {sendError || (phase === "error" && detailError) ? (
        <p className="mt-1 px-1 text-xs text-rose-600">
          {sendError || detailError}
        </p>
      ) : null}
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
