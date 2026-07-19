import { useEffect, useMemo } from "react";
import { PanelRight } from "lucide-react";
import { ProjectHeader } from "./ProjectHeader";
import {
  ConversationList,
  ConversationListRail,
} from "./ConversationList";
import { Timeline, TimelineEmpty } from "./Timeline";
import { SessionInspector } from "./SessionInspector";
import { ProjectBoard } from "./ProjectBoard";
import { SessionsView } from "./SessionsView";
import { CreateProjectEmpty } from "./CreateProjectEmpty";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";

export function WorkView() {
  const projectView = useUiStore((s) => s.projectView);
  const detailsOpen = useUiStore((s) => s.detailsOpen);
  const toggleDetails = useUiStore((s) => s.toggleDetails);
  const projectId = useUiStore((s) => s.projectId);
  const conversationId = useUiStore((s) => s.conversationId);
  const selectProject = useUiStore((s) => s.selectProject);
  const selectConversation = useUiStore((s) => s.selectConversation);
  const listCollapsed = useUiStore((s) => s.conversationListCollapsed);
  const lastConversationByProject = useUiStore(
    (s) => s.lastConversationByProject,
  );
  const projects = useWorkspaceStore((s) => s.projects);
  const conversations = useWorkspaceStore((s) => s.conversations);
  const conversationsStatusByProject = useWorkspaceStore(
    (s) => s.conversationsStatusByProject,
  );

  // Resolve project for this paint: do not wait a frame for App's select effect.
  const resolvedProjectId = useMemo(() => {
    if (projectId && projects.some((p) => p.id === projectId)) return projectId;
    return projects[0]?.id ?? "";
  }, [projectId, projects]);

  // Keep ui-store selection in sync when we had to fall back to projects[0].
  useEffect(() => {
    if (resolvedProjectId && resolvedProjectId !== projectId) {
      selectProject(resolvedProjectId);
    }
  }, [resolvedProjectId, projectId, selectProject]);

  const listStatus = resolvedProjectId
    ? conversationsStatusByProject[resolvedProjectId]
    : undefined;
  const listPhase = listStatus?.phase ?? "idle";

  // Auto-select conversation when list is ready (not while still loading/idle).
  useEffect(() => {
    if (!resolvedProjectId) return;
    if (listPhase === "loading" || listPhase === "idle") return;
    const list = conversations.filter(
      (c) => c.projectId === resolvedProjectId,
    );
    if (list.length === 0) {
      if (conversationId) selectConversation(null);
      return;
    }
    if (conversationId && list.some((c) => c.id === conversationId)) return;
    const remembered = lastConversationByProject[resolvedProjectId];
    const pick =
      (remembered && list.find((c) => c.id === remembered)?.id) || list[0]!.id;
    selectConversation(pick);
  }, [
    resolvedProjectId,
    conversations,
    conversationId,
    lastConversationByProject,
    selectConversation,
    listPhase,
  ]);

  if (projects.length === 0) {
    return <CreateProjectEmpty variant="full" />;
  }

  if (!resolvedProjectId) {
    return (
      <div className="flex flex-1 items-center justify-center bg-surface text-[13px] text-ink-muted">
        Select a project to get started.
      </div>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface">
      <ProjectHeader projectId={resolvedProjectId} />
      {projectView === "board" ? (
        <ProjectBoard key={resolvedProjectId} projectId={resolvedProjectId} />
      ) : projectView === "sessions" ? (
        <ErrorBoundary label="sessions">
          <SessionsView key={resolvedProjectId} projectId={resolvedProjectId} />
        </ErrorBoundary>
      ) : (
        <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
          {listCollapsed ? (
            <ConversationListRail />
          ) : (
            <ConversationList
              key={resolvedProjectId}
              projectId={resolvedProjectId}
            />
          )}
          {conversationId ? (
            <Timeline key={conversationId} conversationId={conversationId} />
          ) : (
            <TimelineEmpty />
          )}
          {detailsOpen && conversationId ? (
            <SessionInspector conversationId={conversationId} />
          ) : conversationId ? (
            <button
              type="button"
              onClick={toggleDetails}
              title="Show inspector"
              className="flex w-10 shrink-0 flex-col items-center border-l border-ink/5 bg-surface pt-3 text-ink-muted hover:bg-surface-hover hover:text-ink"
            >
              <PanelRight className="h-4 w-4" />
            </button>
          ) : null}
        </div>
      )}
    </div>
  );
}
