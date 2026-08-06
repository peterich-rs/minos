import { useEffect, useRef } from "react";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { WorkTimelineShell } from "@/shared/ui/WorkChrome";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { TimelineHeader } from "./TimelineHeader";

const EMPTY_SESSIONS: ProjectSession[] = [];

/**
 * Declarative conversation timeline: parent passes `conversationId` (and
 * ideally `key={conversationId}`). This view owns detail init for that id;
 * children subscribe to workspace state for their slices.
 */
export function Timeline({ conversationId }: { conversationId: string }) {
  const conversations = useWorkspaceStore((s) => s.conversations);
  const loadTimeline = useWorkspaceStore((s) => s.loadTimeline);
  const markConversationRead = useWorkspaceStore((s) => s.markConversationRead);
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

  const conversation = conversations.find((c) => c.id === conversationId);
  const phase = timelineStatus?.phase ?? "idle";
  /** One quiet recovery per error generation — no error→load→error poll loop. */
  const errorRecoveryGenRef = useRef<number | null>(null);

  // Open/select path: set focus + clear unread (not loadTimeline's job).
  useEffect(() => {
    markConversationRead(conversationId);
  }, [conversationId, markConversationRead, bootEpoch]);

  // Init: hydrate Timeline only (messages). Inspector hydrates independently when
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

  // Agent-result arrives via Hub WS / daemon conversation_event + live-ingress
  // single quiet loadTimeline. One-shot quiet recovery only when degraded and
  // live push is off (no generation-loop re-entry).
  useEffect(() => {
    if (source !== "daemon") return;
    if (livePush) return;
    if (phase !== "error") {
      errorRecoveryGenRef.current = null;
      return;
    }
    const gen = timelineStatus?.generation ?? 0;
    if (errorRecoveryGenRef.current === gen) return;
    errorRecoveryGenRef.current = gen;
    void loadTimeline(conversationId, { quiet: true });
  }, [
    conversationId,
    source,
    livePush,
    phase,
    timelineStatus?.generation,
    loadTimeline,
  ]);

  if (!conversation) {
    return (
      <div className="flex h-full min-h-0 flex-1 flex-col items-center justify-center gap-3 bg-canvas-soft/30 px-6 text-center text-sm text-ink-muted">
        <p>Conversation not found in the current project list.</p>
        {source === "daemon" ? (
          <button
            type="button"
            onClick={() => void loadTimeline(conversationId)}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-white shadow-sm"
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
    // Shell classes live in WorkTimelineShell (Desktop + Web SSOT).
    <WorkTimelineShell
      header={
        <TimelineHeader
          conversationId={conversationId}
          conversation={conversation}
          sessionCount={sessions.length}
        />
      }
      composer={<Composer conversationId={conversationId} />}
    >
      <MessageList conversationId={conversationId} />
    </WorkTimelineShell>
  );
}

export { TimelineEmpty } from "./TimelineEmpty";
