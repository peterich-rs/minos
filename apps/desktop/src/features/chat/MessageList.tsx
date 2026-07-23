import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ArrowDown, Loader2 } from "lucide-react";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import { followContentKey } from "@/shared/lib/stick-to-bottom";
import { useStickToBottom } from "@/shared/lib/use-stick-to-bottom";
import { sortTimelineMessages } from "@/shared/lib/timeline-order";
import { nextEnterAnimationIds } from "@/shared/lib/enter-animation";
import {
  EMPTY_MESSAGE_HISTORY,
  MESSAGE_AUTOFILL_SLACK_PX,
} from "@/shared/lib/message-history";
import {
  captureItemScrollAnchor,
  queryScrollItem,
  restoreItemScrollAnchor,
  type ItemScrollAnchor,
} from "@/shared/lib/scroll-restore";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  formatDayDividerLabel,
  isMessageGroupContinuation,
  shouldShowDayDivider,
} from "./lib/message-grouping";
import { MessageRow } from "./MessageRow";

/** Stable empty snapshot for Zustand selectors (never allocate in getSnapshot). */
const EMPTY_MESSAGES: TimelineMessage[] = [];

/**
 * Scrollable conversation message list: stick-to-bottom, load-older,
 * enter-animation gate, and jump-to-latest.
 */
export function MessageList({ conversationId }: { conversationId: string }) {
  const messagesRaw = useWorkspaceStore(
    (s) => s.messagesByConversation[conversationId] ?? EMPTY_MESSAGES,
  );
  const hasCachedMessages = useWorkspaceStore(
    (s) => conversationId in s.messagesByConversation,
  );
  const timelineStatus = useWorkspaceStore(
    (s) => s.timelineStatusByConversation[conversationId],
  );
  const loadTimeline = useWorkspaceStore((s) => s.loadTimeline);
  const loadOlderMessages = useWorkspaceStore((s) => s.loadOlderMessages);
  const source = useWorkspaceStore((s) => s.source);
  const hasOlder = useWorkspaceStore(
    (s) =>
      s.messageHistoryByConversation[conversationId]?.hasOlder ?? false,
  );
  const loadingOlder = useWorkspaceStore(
    (s) =>
      s.messageHistoryByConversation[conversationId]?.loadingOlder ?? false,
  );
  const firstLoadedSeq = useWorkspaceStore(
    (s) =>
      s.messageHistoryByConversation[conversationId]?.firstLoadedSeq ??
      EMPTY_MESSAGE_HISTORY.firstLoadedSeq,
  );

  const messages = useMemo(
    () => sortTimelineMessages(messagesRaw),
    [messagesRaw],
  );
  const messageById = useMemo(() => {
    const map = new Map<string, TimelineMessage>();
    for (const m of messages) map.set(m.id, m);
    return map;
  }, [messages]);
  const phase = timelineStatus?.phase ?? "idle";
  const detailError = timelineStatus?.error;

  const seenMessageIdsRef = useRef<Set<string>>(new Set());
  const [animateIds, setAnimateIds] = useState<Set<string>>(() => new Set());
  const pendingOlderRestoreRef = useRef<{
    anchor: ItemScrollAnchor;
    firstLoadedSeqBefore: number;
  } | null>(null);
  const pinSuspendedRef = useRef(false);
  const olderInFlightRef = useRef(false);
  const topSentinelRef = useRef<HTMLDivElement>(null);

  // Gate enter-animation: first paint / bulk load never blank the whole list.
  useEffect(() => {
    // Conversation switch: reset seen set so the next list is treated as first paint.
    seenMessageIdsRef.current = new Set();
    setAnimateIds(new Set());
    pendingOlderRestoreRef.current = null;
    pinSuspendedRef.current = false;
    olderInFlightRef.current = false;
  }, [conversationId]);

  useEffect(() => {
    const ids = messages.map((m) => m.id);
    const { nextSeen, animateIds: nextAnimate } = nextEnterAnimationIds(
      seenMessageIdsRef.current,
      ids,
    );
    seenMessageIdsRef.current = nextSeen;
    // Avoid re-render when nothing new entered (empty Set → empty Set).
    setAnimateIds((prev) => {
      if (prev.size === 0 && nextAnimate.size === 0) return prev;
      return nextAnimate;
    });
  }, [messages]);

  const contentKey = useMemo(
    () =>
      followContentKey(
        messages.map((m) => ({
          id: m.id,
          seq: m.messageSeq,
          kind: m.kind,
          text: m.body,
        })),
      ),
    [messages],
  );
  const {
    scrollRef,
    contentRef,
    following,
    showJumpToLatest,
    followingRef,
    jumpToLatest,
    markProgrammatic,
    cancelScheduledPin,
  } = useStickToBottom({
    contentKey,
    resetKey: conversationId,
    pinSuspendedRef,
  });

  const loadOlder = useCallback(async () => {
    if (source !== "daemon") return;
    const hist =
      useWorkspaceStore.getState().messageHistoryByConversation[
        conversationId
      ] ?? EMPTY_MESSAGE_HISTORY;
    if (!hist.hasOlder || hist.loadingOlder || olderInFlightRef.current) {
      return;
    }
    olderInFlightRef.current = true;

    const el = scrollRef.current;
    const content = contentRef.current;
    const shouldRestore = !followingRef.current;
    const current =
      useWorkspaceStore.getState().messagesByConversation[conversationId] ??
      [];
    const seqBefore = hist.firstLoadedSeq;
    if (
      shouldRestore &&
      el &&
      content &&
      current.length > 0 &&
      seqBefore != null
    ) {
      const topId = current[0]!.id;
      const itemEl = queryScrollItem(content, topId);
      const anchor = captureItemScrollAnchor(el, topId, itemEl);
      if (anchor) {
        pendingOlderRestoreRef.current = {
          anchor,
          firstLoadedSeqBefore: seqBefore,
        };
      } else {
        pendingOlderRestoreRef.current = null;
      }
    } else {
      pendingOlderRestoreRef.current = null;
    }

    try {
      await loadOlderMessages(conversationId);
    } catch {
      pendingOlderRestoreRef.current = null;
    } finally {
      olderInFlightRef.current = false;
    }
  }, [
    source,
    conversationId,
    loadOlderMessages,
    scrollRef,
    contentRef,
    followingRef,
  ]);

  // Restore viewport after older page lands (firstLoadedSeq decreased).
  useLayoutEffect(() => {
    const pending = pendingOlderRestoreRef.current;
    if (!pending) return;
    if (firstLoadedSeq == null) return;
    if (firstLoadedSeq >= pending.firstLoadedSeqBefore) return;

    if (followingRef.current) {
      pendingOlderRestoreRef.current = null;
      return;
    }

    const el = scrollRef.current;
    const content = contentRef.current;
    if (!el || !content) {
      pendingOlderRestoreRef.current = null;
      return;
    }

    const itemEl = queryScrollItem(content, pending.anchor.itemId);
    cancelScheduledPin();
    pinSuspendedRef.current = true;
    markProgrammatic(120);
    restoreItemScrollAnchor(el, itemEl, pending.anchor);
    pendingOlderRestoreRef.current = null;
    requestAnimationFrame(() => {
      pinSuspendedRef.current = false;
    });
  }, [
    firstLoadedSeq,
    messages,
    scrollRef,
    contentRef,
    followingRef,
    markProgrammatic,
    cancelScheduledPin,
  ]);

  // Silent backfill while following when the tail does not fill the viewport.
  useEffect(() => {
    if (source !== "daemon") return;
    if (!following) return;
    if (phase !== "ready") return;
    if (!hasOlder || loadingOlder) return;
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollHeight <= el.clientHeight + MESSAGE_AUTOFILL_SLACK_PX) {
      void loadOlder();
    }
  }, [
    source,
    following,
    phase,
    hasOlder,
    loadingOlder,
    firstLoadedSeq,
    messages.length,
    loadOlder,
    scrollRef,
  ]);

  // Prefetch older via top sentinel (manual scroll only).
  useEffect(() => {
    const root = scrollRef.current;
    const sentinel = topSentinelRef.current;
    if (!root || !sentinel || typeof IntersectionObserver === "undefined") {
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        if (followingRef.current) return;
        void loadOlder();
      },
      { root, rootMargin: "120px 0px 0px 0px", threshold: 0 },
    );
    io.observe(sentinel);
    return () => io.disconnect();
  }, [conversationId, loadOlder, scrollRef, followingRef, hasOlder]);

  return (
    <>
      <div
        ref={scrollRef}
        className="scrollbar-thin min-h-0 flex-1 overflow-y-auto overscroll-y-none px-5 py-5"
      >
        <div ref={contentRef} className="space-y-4">
          {hasOlder ? (
            <div
              ref={topSentinelRef}
              className="h-px w-full shrink-0"
              aria-hidden
            />
          ) : null}
          {loadingOlder ? (
            <div className="flex justify-center py-1" aria-hidden>
              <Loader2 className="h-3.5 w-3.5 animate-spin text-ink-muted/50" />
            </div>
          ) : null}
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
                onClick={() => void loadTimeline(conversationId)}
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
            messages.map((message, index) => {
              const prev = index > 0 ? messages[index - 1] : undefined;
              const showDay = shouldShowDayDivider(prev, message);
              const grouped = isMessageGroupContinuation(prev, message);
              // Day divider is a sibling of the scroll-identity wrapper so
              // prepend restore / queryScrollItem only measure the message row.
              return (
                <Fragment key={message.id}>
                  {showDay && message.createdAtMs ? (
                    <DayDivider ms={message.createdAtMs} />
                  ) : null}
                  <div data-scroll-id={message.id}>
                    <MessageRow
                      message={message}
                      conversationId={conversationId}
                      replyParent={
                        message.replyToMessageId
                          ? messageById.get(message.replyToMessageId)
                          : undefined
                      }
                      animateIn={animateIds.has(message.id)}
                      groupedWithPrevious={grouped}
                    />
                  </div>
                </Fragment>
              );
            })
          )}
        </div>
      </div>

      {showJumpToLatest ? (
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
    </>
  );
}

function DayDivider({ ms }: { ms: number }) {
  const label = formatDayDividerLabel(ms);
  return (
    <div className="flex items-center gap-3 py-2">
      <div className="h-px flex-1 bg-ink/8" aria-hidden />
      <time
        dateTime={new Date(ms).toISOString().slice(0, 10)}
        className="shrink-0 text-[11px] font-medium text-ink-muted"
      >
        {label}
      </time>
      <div className="h-px flex-1 bg-ink/8" aria-hidden />
    </div>
  );
}
