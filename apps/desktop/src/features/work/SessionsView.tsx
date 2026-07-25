import { useCallback, useEffect, useMemo, useState } from "react";
import { Bot } from "lucide-react";
import {
  useStableArrayShallow,
  useStableSet,
} from "@/shared/hooks/useStableReference";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { groupSessionsByConversation } from "@/shared/lib/session-list-group";
import { SessionListPane } from "./SessionListPane";
import {
  resolveSessionForView,
  sessionBelongsToProject,
} from "./lib/session-view-resolve";
import { TranscriptPane } from "./ui/TranscriptPane";

/** Stable empty list for Zustand selectors (never allocate in getSnapshot). */
const EMPTY_PROJECT_SESSIONS: ProjectSession[] = [];

/**
 * Project sessions tab — parent passes projectId (and key).
 * List load is owned here; transcript load is owned by TranscriptPane.
 *
 * Left rail is conversation-grouped (Codex-style folder → runs): each
 * conversation collapses independently; sessions show live progress.
 */
export function SessionsView({ projectId }: { projectId: string }) {
  const selectedSessionId = useUiStore((s) => s.selectedSessionId);
  const conversationId = useUiStore((s) => s.conversationId);
  const projectView = useUiStore((s) => s.projectView);
  const selectSession = useUiStore((s) => s.selectSession);
  const openConversation = useUiStore((s) => s.openConversation);
  const listCollapsed = useUiStore((s) => s.sessionsListCollapsed);
  const toggleSessionsList = useUiStore((s) => s.toggleSessionsList);

  // SessionList for this project only (no global projectSessions mirror).
  const projectSessions = useWorkspaceStore(
    (s) => s.projectSessionsByProject[projectId] ?? EMPTY_PROJECT_SESSIONS,
  );
  const sessionsByConversation = useWorkspaceStore(
    (s) => s.sessionsByConversation,
  );
  const conversations = useWorkspaceStore((s) => s.conversations);
  const listStatus = useWorkspaceStore(
    (s) => s.projectSessionsStatusByProject[projectId],
  );
  const loadProjectSessions = useWorkspaceStore((s) => s.loadProjectSessions);
  const source = useWorkspaceStore((s) => s.source);
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  const livePush = useWorkspaceStore((s) => s.livePush);

  /** Conversation ids the user has collapsed (default: all expanded). */
  const [collapsedConvIdsState, setCollapsedConvIds] = useState<Set<string>>(
    () => new Set(),
  );
  const collapsedConvIds = useStableSet(collapsedConvIdsState);

  // conversationId → projectId, so deep-link resolution can reject a foreign
  // project's session even during the project-sessions loading race.
  const conversationProjectById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of conversations) {
      if (c.projectId) map[c.id] = c.projectId;
    }
    return map;
  }, [conversations]);

  // Merge deep-linked session into the list only if it belongs to this project.
  const displaySessions = useStableArrayShallow(
    useMemo(() => {
      if (!selectedSessionId) return projectSessions;
      if (sessionBelongsToProject(selectedSessionId, projectSessions)) {
        return projectSessions;
      }
      const fallback = resolveSessionForView(
        selectedSessionId,
        projectId,
        projectSessions,
        sessionsByConversation,
        conversationProjectById,
      );
      if (!fallback) return projectSessions;
      // Still only inject while list is empty (deep-link race), never a foreign id.
      if (projectSessions.length > 0) return projectSessions;
      return [fallback, ...projectSessions];
    }, [
      projectSessions,
      selectedSessionId,
      sessionsByConversation,
      conversationProjectById,
      projectId,
    ]),
  );

  const groups = useStableArrayShallow(
    useMemo(
      () => groupSessionsByConversation(displaySessions),
      [displaySessions],
    ),
  );

  const handleToggleConversation = useCallback((conversationId: string) => {
    setCollapsedConvIds((prev) => {
      const next = new Set(prev);
      if (next.has(conversationId)) next.delete(conversationId);
      else next.add(conversationId);
      return next;
    });
  }, []);

  // Load project sessions when:
  // - projectId / bootEpoch changes, or
  // - user switches to Sessions tab (keep-alive may still be mounted with a
  //   stale list after agents started under other conversations).
  // Quiet when we already have rows so remount does not flash "Loading…".
  useEffect(() => {
    if (source !== "daemon") return;
    // Keep-alive under Conversations: do not spam list_project_sessions on every
    // conversation click — only hydrate while Sessions tab is active (or first
    // mount when already on Sessions).
    if (projectView !== "sessions") return;
    const hasRows =
      (useWorkspaceStore.getState().projectSessionsByProject[projectId]
        ?.length ?? 0) > 0;
    void loadProjectSessions(projectId, { quiet: hasRows });
  }, [projectId, source, loadProjectSessions, bootEpoch, projectView]);

  // Project switch: reset folder collapse. (selectProject already clears
  // selectedSessionId; do not re-clear here.)
  useEffect(() => {
    setCollapsedConvIds(new Set());
  }, [projectId]);

  // Auto-select a root session when nothing valid is selected.
  // Prefer the session under the conversation the user just left (Conversations tab).
  //
  // Only while the Sessions tab is active: keep-alive leaves this view mounted
  // under Conversations, and re-selecting / clearing here would undo
  // SessionInspector's selectSession(id) (SessionDetail) and
  // "← Back to conversation" (selectSession(null)).
  useEffect(() => {
    if (projectView !== "sessions") return;
    if (selectedSessionId) {
      // Valid if in project list OR same-project deep-link (inspector / openSession).
      if (
        resolveSessionForView(
          selectedSessionId,
          projectId,
          projectSessions,
          sessionsByConversation,
          conversationProjectById,
        )
      ) {
        return;
      }
      // Foreign or stale id — do not wait forever on empty load.
      const phase = listStatus?.phase ?? "idle";
      if (phase === "loading" || phase === "idle") return;
    }
    const fromConversation = conversationId
      ? displaySessions.find(
          (s) => s.conversationId === conversationId && !s.parentId,
        )
      : undefined;
    const firstRoot =
      fromConversation ?? groups[0]?.roots[0] ?? displaySessions[0];
    if (firstRoot) selectSession(firstRoot.id);
    else if (selectedSessionId) selectSession(null);
  }, [
    projectView,
    groups,
    displaySessions,
    selectedSessionId,
    selectSession,
    listStatus?.phase,
    conversationId,
    projectId,
    projectSessions,
    sessionsByConversation,
    conversationProjectById,
  ]);

  // Keep the conversation of the selected session expanded.
  useEffect(() => {
    if (!selectedSessionId) return;
    const session = displaySessions.find((s) => s.id === selectedSessionId);
    if (!session) return;
    setCollapsedConvIds((prev) => {
      if (!prev.has(session.conversationId)) return prev;
      const next = new Set(prev);
      next.delete(session.conversationId);
      return next;
    });
  }, [selectedSessionId, displaySessions]);

  // Degraded quiet poll of project sessions when live push is off.
  // Live path relies on manager events; no interval while livePush is healthy.
  useEffect(() => {
    if (source !== "daemon") return;
    if (livePush) return;
    const live = displaySessions.some(
      (s) => s.status === "running" || s.status === "needs_approval",
    );
    if (!live && listStatus?.phase !== "error") return;
    const id = window.setInterval(() => {
      void loadProjectSessions(projectId, { quiet: true });
    }, 2000);
    return () => window.clearInterval(id);
  }, [
    projectId,
    source,
    livePush,
    displaySessions,
    listStatus?.phase,
    loadProjectSessions,
  ]);

  const selected = resolveSessionForView(
    selectedSessionId,
    projectId,
    displaySessions,
    sessionsByConversation,
    conversationProjectById,
  );
  const phase = listStatus?.phase ?? "idle";

  return (
    <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
      {listCollapsed ? (
        <div className="flex w-10 shrink-0 flex-col items-center border-r border-ink/5 bg-surface pt-2.5">
          <button
            type="button"
            title="Expand sessions list"
            onClick={toggleSessionsList}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-ink-muted hover:bg-surface-hover hover:text-ink"
          >
            <Bot className="h-4 w-4" />
          </button>
        </div>
      ) : (
        <SessionListPane
          groups={groups}
          projectSessionCount={projectSessions.length}
          phase={phase}
          error={listStatus?.error}
          selectedSessionId={selectedSessionId}
          collapsedConvIds={collapsedConvIds}
          onToggleConversation={handleToggleConversation}
          onSelectSession={selectSession}
          onRetry={() => void loadProjectSessions(projectId)}
          onCollapseList={toggleSessionsList}
        />
      )}

      {selectedSessionId && selected ? (
        <TranscriptPane
          key={selectedSessionId}
          sessionId={selectedSessionId}
          session={selected}
          onBackToConversation={() =>
            openConversation(selected.conversationId)
          }
        />
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center bg-surface text-sm text-ink-muted">
          Select an agent session to view its full transcript.
        </div>
      )}
    </div>
  );
}
