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
import { MessageAuthorText, MessageHeaderRow } from "./MessageHeader";
import { MessageTimestamp } from "./MessageTimestamp";
import { useReactionStore } from "./reaction-store";

/**
 * Slack/Buzz-style message row: full-width, left-aligned for every author.
 * No left/right bubble split — user and agent share the same grammar.
 */
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
          "mx-auto max-w-md rounded-xl bg-surface-muted px-3 py-2 text-center text-xs text-ink-muted",
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
  const sessionShort = message.sessionId
    ? shortSessionId(message.sessionId)
    : undefined;
  const agentLabel = agent?.label ?? message.agent ?? "Agent";
  const authorLabel = isUser ? "You" : agentLabel;
  const avatarTone = isUser ? "slate" : (agent?.tone ?? "slate");

  if (message.kind === "tool_summary") {
    return (
      <div
        className={cn(
          "flex w-full items-center gap-2 rounded-xl border border-ink/5 bg-surface-muted/80 px-3 py-2 text-left text-xs text-ink-secondary",
          enterClass,
        )}
      >
        <Wrench className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 truncate">{message.body}</span>
        <span className="shrink-0 text-ink-muted">{message.time}</span>
      </div>
    );
  }

  const isContinuation = groupedWithPrevious;
  const fullTitle =
    message.createdAtMs && message.createdAtMs > 0
      ? new Date(message.createdAtMs).toLocaleString()
      : undefined;

  const avatarGutter = isContinuation ? (
    <div
      aria-hidden
      className={cn(
        "flex w-9 shrink-0 items-start justify-end pt-0.5 self-stretch",
      )}
    >
      <MessageTimestamp
        time={message.time}
        title={fullTitle}
        className="opacity-0 transition-opacity group-hover/message:opacity-100 group-focus-within/message:opacity-100"
      />
    </div>
  ) : (
    <div className="flex w-9 shrink-0 items-start justify-center pt-0.5">
      <Avatar name={authorLabel} tone={avatarTone} size="md" />
    </div>
  );

  const headerNode = isContinuation ? null : (
    <MessageHeaderRow>
      {message.sessionId && !isUser ? (
        <button
          type="button"
          title={`Open ${agentLabel} #${sessionShort} transcript`}
          onClick={() =>
            openSessionTranscript(message.sessionId!, conversationId)
          }
          className="inline-flex min-w-0 items-center gap-1 rounded-md hover:bg-surface-hover"
        >
          <MessageAuthorText hoverUnderline as="span">
            {authorLabel}
          </MessageAuthorText>
          {sessionShort ? (
            <span className="font-mono text-2xs font-normal text-ink-muted">
              #{sessionShort}
            </span>
          ) : null}
        </button>
      ) : (
        <MessageAuthorText as="h3">{authorLabel}</MessageAuthorText>
      )}
      <MessageTimestamp time={message.time} title={fullTitle} />
      {isSending ? (
        <span className="text-2xs font-medium uppercase tracking-wide text-ink-muted/80">
          Sending
        </span>
      ) : null}
      {isFailed ? (
        <span className="text-2xs font-medium text-rose-600">Failed</span>
      ) : null}
    </MessageHeaderRow>
  );

  return (
    <article
      className={cn(
        "group/message relative z-10 flex gap-2.5 rounded-2xl px-2 py-1 transition-colors",
        "mx-1 hover:bg-surface-hover/80 focus-within:bg-surface-hover/80",
        isContinuation ? "items-center" : "items-start",
        enterClass,
        isContinuation && "-mt-0.5",
      )}
      data-message-id={message.id}
      data-testid="message-row"
    >
      {avatarGutter}

      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {headerNode}

        {message.replyToMessageId ? (
          <ReplyPreview
            replyToMessageId={message.replyToMessageId}
            replyParent={replyParent}
          />
        ) : null}

        <div
          className={cn(
            "relative max-w-full text-sm leading-relaxed text-ink",
            isContinuation ? "mt-0" : "-mt-0.5",
            isSending && "opacity-70",
          )}
        >
          <div className="flex items-start gap-1.5">
            {isUser && isFailed ? (
              <button
                type="button"
                onClick={handleRetry}
                title="Message failed to send — click to retry"
                className="mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-rose-600 text-xs font-bold leading-none text-white shadow-sm hover:bg-rose-700"
                aria-label="Retry failed message"
              >
                !
              </button>
            ) : null}
            <div className="min-w-0 flex-1">
              <MarkdownText
                text={message.body}
                tone="default"
                className="text-sm"
              />
            </div>
          </div>
        </div>

        <MessageReactions
          groups={reactions}
          onToggle={(emoji) => toggleReaction(message.id, emoji)}
        />
      </div>

      <div className="absolute right-2 top-0 z-10 -translate-y-1/2 sm:top-1 sm:translate-y-0 sm:group-hover/message:top-0 sm:group-hover/message:-translate-y-1/2">
        <MessageActionBar
          onReply={() => setReplyTo(conversationId, message.id)}
          onReact={(emoji) => toggleReaction(message.id, emoji)}
        />
      </div>
    </article>
  );
});

const EMPTY_REACTIONS: never[] = [];
