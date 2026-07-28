import { useEffect } from "react";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { TimelineHeader } from "./TimelineHeader";

const EMPTY_SESSIONS: ProjectSession[] = [];
const EMPTY_MESSAGES: never[] = [];

/**
 * Declarative conversation timeline: parent passes `conversationId` (and
 * ideally `key={conversationId}`). This view owns detail init for that id;
 * children subscribe to workspace state for their slices.
 */
export function Timeline({ conversationId }: { conversationId: string }) {
  const conversations = useWorkspaceStore((s) => s.conversations);
  const loadTimeline = useWorkspaceStore((s) => s.loadTimeline);
  const refreshConversationGitStatus = useWorkspaceStore(
    (s) => s.refreshConversationGitStatus,
  );
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const livePush = useWorkspaceStore((s) => s.livePush);
  const sessions = useWorkspaceStore(
    (s) => s.sessionsByConversation[conversationId] ?? EMPTY_SESSIONS,
  );
  const timelineStatus = useWorkspaceStore(
    (s) => s.timelineStatusByConversation[conversationId],
  );
  const messagesLength = useWorkspaceStore(
    (s) =>
      (s.messagesByConversation[conversationId] ?? EMPTY_MESSAGES).length,
  );

  const conversation = conversations.find((c) => c.id === conversationId);
  const phase = timelineStatus?.phase ?? "idle";

  // Init: load Timeline only (messages). Inspector hydrates independently when
  // details panel opens or @-mention needs sessions.
  useEffect(() => {
    if (source !== "daemon") return;
    void loadTimeline(conversationId);
    // Live git dirty/branch for header chips (best-effort).
    void refreshConversationGitStatus(conversationId);
  }, [
    conversationId,
    source,
    loadTimeline,
    refreshConversationGitStatus,
    bootEpoch,
  ]);

  // Degraded quiet poll of Timeline only when live push is off.
  // Live path relies on applyConversationEvent → conditional loadTimeline.
  const hasLiveSession = sessions.some(
    (s) => s.status === "running" || s.status === "needs_approval",
  );
  const expectHistoryEmpty =
    (conversation?.messageCount ?? 0) > 0 && messagesLength === 0;
  useEffect(() => {
    if (source !== "daemon") return;
    if (livePush) return;
    const needPoll =
      hasLiveSession || phase === "error" || expectHistoryEmpty;
    if (!needPoll) return;
    const id = window.setInterval(() => {
      void loadTimeline(conversationId, { quiet: true });
    }, 2500);
    return () => window.clearInterval(id);
  }, [
    conversationId,
    source,
    livePush,
    hasLiveSession,
    expectHistoryEmpty,
    phase,
    loadTimeline,
  ]);

  if (!conversation) {
    return (
      <div className="flex h-full min-h-0 flex-1 flex-col items-center justify-center gap-3 bg-surface px-6 text-center text-sm text-ink-muted">
        <p>Conversation not found in the current project list.</p>
        {source === "daemon" ? (
          <button
            type="button"
            onClick={() => void loadTimeline(conversationId)}
            className="rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface"
          >
            Retry load
          </button>
        ) : null}
      </div>
    );
  }

  return (
    // Fill the resizable panel / flex parent so the composer stays docked at the
    // bottom (WeChat-style): only the message list scrolls, never the whole pane.
    <section className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface">
      <TimelineHeader
        conversationId={conversationId}
        conversation={conversation}
        sessionCount={sessions.length}
      />
      <MessageList conversationId={conversationId} />
      <Composer conversationId={conversationId} />
    </section>
  );
}

export { TimelineEmpty } from "./TimelineEmpty";
