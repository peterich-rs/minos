import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ArrowDown, Loader2 } from "lucide-react";
import { VList, type VListHandle } from "virtua";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import { followContentKey } from "@/shared/lib/stick-to-bottom";
import { nextEnterAnimationIds } from "@/shared/lib/enter-animation";
import { EMPTY_MESSAGE_HISTORY } from "@/shared/lib/message-history";
import { sortTimelineMessages } from "@/shared/lib/timeline-order";
import { useWorkspaceStore } from "@/store/workspace-store";
import { formatDayDividerLabel } from "./lib/message-grouping";
import {
  buildVirtualTimelineItems,
  type VirtualTimelineItem,
} from "./lib/virtual-timeline-items";
import { MessageRow } from "./MessageRow";

/** Stable empty snapshot for Zustand selectors (never allocate in getSnapshot). */
const EMPTY_MESSAGES: TimelineMessage[] = [];

/**
 * Virtualized conversation message list (virtua):
 * stick-to-bottom, load-older on scroll top, enter-animation gate, jump-to-latest.
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
  const virtualItems = useMemo(
    () => buildVirtualTimelineItems(messages),
    [messages],
  );
  const phase = timelineStatus?.phase ?? "idle";
  const detailError = timelineStatus?.error;

  const listRef = useRef<VListHandle>(null);
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  const [shift, setShift] = useState(false);
  const olderInFlightRef = useRef(false);
  const prevFirstSeqRef = useRef<number | null | undefined>(undefined);
  const seenMessageIdsRef = useRef<Set<string>>(new Set());
  const [animateIds, setAnimateIds] = useState<Set<string>>(() => new Set());

  const setFollowingState = useCallback((next: boolean) => {
    followingRef.current = next;
    setFollowing(next);
  }, []);

  useEffect(() => {
    seenMessageIdsRef.current = new Set();
    setAnimateIds(new Set());
    olderInFlightRef.current = false;
    prevFirstSeqRef.current = undefined;
    setShift(false);
    setFollowingState(true);
  }, [conversationId, setFollowingState]);

  useEffect(() => {
    const ids = messages.map((m) => m.id);
    const { nextSeen, animateIds: nextAnimate } = nextEnterAnimationIds(
      seenMessageIdsRef.current,
      ids,
    );
    seenMessageIdsRef.current = nextSeen;
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

  // Stick to bottom when following and tail content changes.
  useLayoutEffect(() => {
    if (!followingRef.current) return;
    if (virtualItems.length === 0) return;
    listRef.current?.scrollToIndex(virtualItems.length - 1, { align: "end" });
  }, [contentKey, conversationId, virtualItems.length]);

  // After older page prepends (firstLoadedSeq decreases), keep shift for one frame.
  useLayoutEffect(() => {
    const prev = prevFirstSeqRef.current;
    if (
      firstLoadedSeq != null &&
      prev != null &&
      firstLoadedSeq < prev
    ) {
      setShift(true);
      requestAnimationFrame(() => setShift(false));
    }
    prevFirstSeqRef.current = firstLoadedSeq;
  }, [firstLoadedSeq]);

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
    setShift(true);
    try {
      await loadOlderMessages(conversationId);
    } finally {
      olderInFlightRef.current = false;
      requestAnimationFrame(() => setShift(false));
    }
  }, [source, conversationId, loadOlderMessages]);

  // Silent backfill while following when the virtual list is short.
  useEffect(() => {
    if (source !== "daemon") return;
    if (!following) return;
    if (phase !== "ready") return;
    if (!hasOlder || loadingOlder) return;
    // Cheap autofill when few rows (viewport not full).
    if (messages.length < 24) {
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
  ]);

  const handleScroll = useCallback(
    (offset: number) => {
      const handle = listRef.current;
      if (!handle) return;
      const scrollSize = handle.scrollSize;
      const viewport = handle.viewportSize;
      const distanceFromBottom = scrollSize - viewport - offset;
      const atBottom = distanceFromBottom < 80;
      if (atBottom !== followingRef.current) {
        setFollowingState(atBottom);
      }
      // Prefetch older near top (manual scroll only).
      if (offset < 120 && !followingRef.current) {
        void loadOlder();
      }
    },
    [loadOlder, setFollowingState],
  );

  const jumpToLatest = useCallback(() => {
    setFollowingState(true);
    if (virtualItems.length > 0) {
      listRef.current?.scrollToIndex(virtualItems.length - 1, {
        align: "end",
      });
    }
  }, [setFollowingState, virtualItems.length]);

  const showJumpToLatest = !following && messages.length > 0;

  const renderVirtualItem = (item: VirtualTimelineItem) => {
    if (item.type === "day") {
      return <DayDivider key={item.id} ms={item.ms} />;
    }
    return (
      <div key={item.id} data-scroll-id={item.id}>
        <MessageRow
          message={item.message}
          conversationId={conversationId}
          replyParent={
            item.message.replyToMessageId
              ? messageById.get(item.message.replyToMessageId)
              : undefined
          }
          animateIn={animateIds.has(item.id)}
          groupedWithPrevious={item.groupedWithPrevious}
        />
      </div>
    );
  };

  const emptyOrStatus =
    phase === "loading" && !hasCachedMessages ? (
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
    ) : null;

  return (
    <>
      <div className="relative min-h-0 flex-1">
        {emptyOrStatus ? (
          <div className="px-5 py-5">{emptyOrStatus}</div>
        ) : (
          <VList
            ref={listRef}
            className="scrollbar-thin h-full px-5 py-5"
            shift={shift}
            onScroll={handleScroll}
          >
            {hasOlder || loadingOlder ? (
              <div className="flex justify-center py-2" key="__older-head">
                {loadingOlder ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-ink-muted/50" />
                ) : (
                  <span className="h-px w-full" aria-hidden />
                )}
              </div>
            ) : null}
            {virtualItems.map((item) => renderVirtualItem(item))}
          </VList>
        )}
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


