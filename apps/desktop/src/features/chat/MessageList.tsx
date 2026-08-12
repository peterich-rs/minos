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
import type { TimelineMessage } from "@/shared/domain/collaboration";
import {
  useStableArrayShallow,
  useStableMap,
  useStableSet,
} from "@/shared/hooks/useStableReference";
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

  // Stable refs: sort/build often allocate equal content after quiet polls;
  // keep identity so MessageRow memo + virtua children can bail.
  const messages = useStableArrayShallow(
    useMemo(() => sortTimelineMessages(messagesRaw), [messagesRaw]),
  );
  const messageById = useStableMap(
    useMemo(() => {
      const map = new Map<string, TimelineMessage>();
      for (const m of messages) map.set(m.id, m);
      return map;
    }, [messages]),
  );
  const virtualItems = useStableArrayShallow(
    useMemo(() => buildVirtualTimelineItems(messages), [messages]),
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
  const [animateIdsState, setAnimateIds] = useState<Set<string>>(
    () => new Set(),
  );
  const animateIds = useStableSet(animateIdsState);

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
  // Double-rAF: first layout after flex height resolves (Tauri/WKWebView), then
  // scroll — a single pass often runs while VList clientHeight is still 0.
  useLayoutEffect(() => {
    if (!followingRef.current) return;
    if (virtualItems.length === 0) return;
    const index = virtualItems.length - 1;
    listRef.current?.scrollToIndex(index, { align: "end" });
    const id = requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (!followingRef.current) return;
        listRef.current?.scrollToIndex(index, { align: "end" });
      });
    });
    return () => cancelAnimationFrame(id);
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

  // Memoized row renderer: stable deps so list parent re-renders (scroll
  // following flag, etc.) do not rebuild every MessageRow element tree.
  const renderVirtualItem = useCallback(
    (item: VirtualTimelineItem) => {
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
    },
    [animateIds, conversationId, messageById],
  );

  const emptyOrStatus =
    phase === "loading" && !hasCachedMessages ? (
      <div className="flex flex-col items-center gap-3 py-16 text-center">
        <Loader2 className="h-5 w-5 animate-spin text-primary/70" />
        <p className="text-sm text-ink-muted">Loading messages…</p>
      </div>
    ) : phase === "error" && !hasCachedMessages ? (
      <div className="flex flex-col items-center gap-3 py-16 text-center">
        <p className="text-sm text-status-failed">
          {detailError || "Failed to load messages"}
        </p>
        <button
          type="button"
          onClick={() => void loadTimeline(conversationId)}
          className="rounded-xl bg-primary px-3.5 py-2 text-xs font-semibold text-white shadow-sm hover:opacity-90"
        >
          Retry
        </button>
      </div>
    ) : messages.length === 0 ? (
      <div className="mx-auto max-w-sm py-16 text-center text-sm text-ink-muted">
        <p className="font-medium text-ink-secondary">No messages yet</p>
        <p className="mt-2 leading-relaxed">
          Type{" "}
          <kbd className="rounded-md border border-ink/10 bg-surface px-1.5 py-0.5 font-mono text-xs text-ink shadow-sm">
            @grok
          </kbd>{" "}
          or{" "}
          <kbd className="rounded-md border border-ink/10 bg-surface px-1.5 py-0.5 font-mono text-xs text-ink shadow-sm">
            @codex
          </kbd>{" "}
          to start an agent.
        </p>
      </div>
    ) : null;

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {emptyOrStatus ? (
        <div
          className="min-h-0 flex-1 overflow-y-auto px-5 py-5"
          style={{ flex: "1 1 0%" }}
        >
          {emptyOrStatus}
        </div>
      ) : (
        <VList
          ref={listRef}
          // virtua requires a definite viewport height. flex-basis:0 matches
          // TranscriptPane (Tauri/WKWebView); plain h-full often resolves to 0
          // and yields a scrollbar with no visible rows.
          className="scrollbar-thin min-h-0 px-3 py-4 sm:px-5"
          style={{ flex: "1 1 0%", height: "100%", minHeight: 0 }}
          shift={shift}
          onScroll={handleScroll}
          // Extra buffer so day dividers + tall agent turns stay smooth (px).
          bufferSize={480}
        >
          {hasOlder || loadingOlder ? (
            <div
              className="flex items-center justify-center gap-2 py-3"
              key="__older-head"
            >
              {loadingOlder ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-primary/60" />
                  <span className="text-2xs font-medium text-ink-muted">
                    Loading earlier messages…
                  </span>
                </>
              ) : (
                <span
                  className="h-px w-16 rounded-full bg-ink/10"
                  aria-hidden
                />
              )}
            </div>
          ) : null}
          {virtualItems.map((item) => renderVirtualItem(item))}
          {/* Bottom spacer so last bubble clears jump FAB + composer shadow */}
          <div className="h-2 shrink-0" aria-hidden key="__tail-pad" />
        </VList>
      )}

      {showJumpToLatest ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-3 z-10 flex justify-center">
          <button
            type="button"
            onClick={jumpToLatest}
            className="pointer-events-auto inline-flex animate-message-in items-center gap-1.5 rounded-full border border-ink/10 bg-surface/95 px-3.5 py-2 text-xs font-semibold text-ink shadow-lg backdrop-blur-md transition-colors hover:bg-primary hover:text-white hover:border-primary motion-reduce:animate-none"
          >
            <ArrowDown className="h-3.5 w-3.5" />
            Jump to latest
          </button>
        </div>
      ) : null}
    </div>
  );
}

function DayDivider({ ms }: { ms: number }) {
  const label = formatDayDividerLabel(ms);
  return (
    <div className="flex items-center gap-3 py-3">
      <div className="h-px flex-1 bg-ink/8" aria-hidden="true" />
      <time
        dateTime={new Date(ms).toISOString().slice(0, 10)}
        className="shrink-0 rounded-full border border-ink/8 bg-surface/90 px-2.5 py-0.5 text-2xs font-semibold tabular-nums text-ink-muted shadow-sm backdrop-blur-sm"
      >
        {label}
      </time>
      <div className="h-px flex-1 bg-ink/8" aria-hidden="true" />
    </div>
  );
}


