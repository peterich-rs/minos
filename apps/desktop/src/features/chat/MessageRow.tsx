import { memo, useState } from "react";
import { Wrench } from "lucide-react";
import { agentMeta, type TimelineMessage } from "@/shared/lib/mock-data";
import { shortSessionId, type KnownAgent } from "@/shared/lib/agent-route";
import { Avatar } from "@/shared/ui/Avatar";
import { MarkdownText } from "@/shared/ui/MarkdownText";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";
import { ReplyPreview } from "./ReplyPreview";
import { MessageReactions } from "./MessageReactions";
import { MessageActionBar } from "./MessageActionBar";
import { useReactionStore } from "./reaction-store";

export const MessageRow = memo(function MessageRow({
  message,
  conversationId,
  replyParent,
  animateIn = false,
  /** Hide avatar + agent header when continuing the same author group. */
  groupedWithPrevious = false,
  onRetry,
}: {
  message: TimelineMessage;
  conversationId: string;
  replyParent?: TimelineMessage;
  /** Only newly appended rows play enter animation (never bulk history). */
  animateIn?: boolean;
  groupedWithPrevious?: boolean;
  /** Retry callback for failed user messages (red `!` icon). */
  onRetry?: (messageId: string) => void;
}) {
  const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
  const setReplyTo = useUiStore((s) => s.setReplyTo);
  const retryFailedMessage = useWorkspaceStore((s) => s.retryFailedMessage);
  const reactions = useReactionStore(
    (s) => s.reactionsByMessageId[message.id] ?? EMPTY_REACTIONS,
  );
  const toggleReaction = useReactionStore((s) => s.toggleReaction);
  const [retrying, setRetrying] = useState(false);
  const enterClass = animateIn
    ? "animate-message-in motion-reduce:animate-none"
    : undefined;
  const delivery = message.deliveryStatus;
  // Bubble is in sending state when the row says so, or while a retry is in
  // flight (the store patches deliveryStatus asynchronously).
  const isSending = delivery === "sending" || retrying;
  const isFailed = delivery === "failed" && !retrying;

  const handleRetry = () => {
    if (onRetry) {
      onRetry(message.id);
      return;
    }
    setRetrying(true);
    void retryFailedMessage(conversationId, message.id)
      .catch(() => {
        /* store sets actionError + patches row back to failed */
      })
      .finally(() => setRetrying(false));
  };

  if (message.role === "system") {
    return (
      <div
        className={cn(
          "mx-auto max-w-md rounded-xl bg-surface-muted px-3 py-2 text-center text-[12px] text-ink-muted",
          enterClass,
        )}
      >
        {message.body}
      </div>
    );
  }

  const isUser = message.role === "user";
  const agentKey = message.agent as KnownAgent | undefined;
  const agent = agentKey && agentMeta[agentKey] ? agentMeta[agentKey] : null;
  // TUI labels agent replies as `[OpenCode@b15d06d4]` — surface session short id.
  const sessionShort = message.sessionId
    ? shortSessionId(message.sessionId)
    : undefined;
  const agentLabel = agent?.label ?? message.agent ?? "Agent";

  // Conversation timeline must not invent "approval" cards from free text.
  // Real approvals (permission / plan / opencode question) live on the session
  // transcript with a requestId and wired Allow/Deny — same as TUI.
  // Live run status / open-session CTAs stay in the right-hand Session inspector
  // (TUI parity: do not pollute the chat timeline with session chrome).

  if (message.kind === "tool_summary") {
    return (
      <div
        className={cn(
          "flex w-full items-center gap-2 rounded-xl border border-ink/5 bg-surface-muted/80 px-3 py-2 text-left text-[12px] text-ink-secondary",
          enterClass,
        )}
      >
        <Wrench className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 truncate">{message.body}</span>
        <span className="shrink-0 text-ink-muted">{message.time}</span>
      </div>
    );
  }

  const showHeader = !isUser && !groupedWithPrevious;
  const showAvatar = !isUser && !groupedWithPrevious;

  return (
    <div
      className={cn(
        "group flex gap-2.5",
        enterClass,
        isUser ? "justify-end" : "justify-start",
        // Tighter vertical rhythm for continuation rows.
        groupedWithPrevious && "-mt-2",
      )}
    >
      {!isUser ? (
        showAvatar ? (
          <Avatar name={agentLabel} tone={agent?.tone ?? "slate"} />
        ) : (
          // Spacer keeps bubble aligned when avatar is hidden.
          <div className="w-8 shrink-0" aria-hidden />
        )
      ) : null}
      <div
        className={cn(
          "relative max-w-[78%] space-y-1",
          isUser && "items-end",
        )}
      >
        {showHeader ? (
          <div className="flex items-center gap-1.5 px-1 text-[12px]">
            {message.sessionId ? (
              <button
                type="button"
                title={`Open ${agentLabel} #${sessionShort} transcript`}
                onClick={() =>
                  openSessionTranscript(message.sessionId!, conversationId)
                }
                className="inline-flex min-w-0 items-center gap-1 rounded-md px-0.5 hover:bg-surface-hover"
              >
                <span className="font-medium text-ink">{agentLabel}</span>
                {sessionShort ? (
                  <span className="font-mono text-[11px] font-normal text-ink-muted">
                    #{sessionShort}
                  </span>
                ) : null}
              </button>
            ) : (
              <span className="font-medium text-ink">{agentLabel}</span>
            )}
            {isSending ? (
              <span className="text-[11px] text-ink-muted">sending…</span>
            ) : null}
          </div>
        ) : null}
        {message.replyToMessageId ? (
          <ReplyPreview
            replyToMessageId={message.replyToMessageId}
            replyParent={replyParent}
            isUser={isUser}
          />
        ) : null}
        <div
          className={cn(
            "relative flex items-end gap-1.5",
            isUser && "flex-row-reverse",
          )}
        >
          {isUser && isFailed ? (
            <button
              type="button"
              onClick={handleRetry}
              title="Message failed to send — click to retry"
              className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-rose-600 text-[12px] font-bold leading-none text-white shadow-sm hover:bg-rose-700"
              aria-label="Retry failed message"
            >
              !
            </button>
          ) : null}
          <div className="relative">
            <div
              className={cn(
                "rounded-2xl px-3.5 py-2.5 text-[13.5px] leading-relaxed shadow-sm",
                isUser
                  ? "rounded-br-md bg-bubble-out text-white"
                  : "rounded-bl-md border border-ink/5 bg-surface-muted/60 text-ink",
                isSending && "opacity-70",
              )}
            >
              <MarkdownText
                text={message.body}
                tone={isUser ? "onDark" : "default"}
                className="text-[13.5px]"
              />
            </div>
            <MessageActionBar
              isUser={isUser}
              onReply={() => setReplyTo(conversationId, message.id)}
              onReact={(emoji) => toggleReaction(message.id, emoji)}
              className={cn(
                "absolute -top-3 z-[1]",
                isUser ? "left-0 -translate-x-1" : "right-0 translate-x-1",
              )}
            />
          </div>
        </div>
        <MessageReactions
          groups={reactions}
          onToggle={(emoji) => toggleReaction(message.id, emoji)}
          align={isUser ? "end" : "start"}
        />
        <div
          className={cn(
            "px-1 text-[11px] text-ink-muted",
            isUser && "text-right",
          )}
        >
          {isSending ? "sending…" : isFailed ? "failed" : message.time}
        </div>
      </div>
    </div>
  );
});

const EMPTY_REACTIONS: never[] = [];
