import { useEffect, useMemo } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { PanelRight } from "lucide-react";
import { ProjectHeader } from "./ProjectHeader";
import {
  ConversationList,
  ConversationListRail,
} from "./ConversationList";
import { Timeline, TimelineEmpty } from "@/features/chat/Timeline";
import { SessionInspector } from "./SessionInspector";
import { ProjectBoard } from "./ProjectBoard";
import { SessionsView } from "./SessionsView";
import { CreateProjectEmpty } from "./CreateProjectEmpty";
import { ErrorBoundary } from "@/shared/ui/ErrorBoundary";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { cn } from "@/shared/lib/utils";

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

  // Resolve project for this paint: do not wait a frame for App's select
  // effect. Fall back to the sorted head (attention-first) rather than
  // projects[0] (daemon row order).
  const resolvedProjectId = useMemo(() => {
    if (projectId && projects.some((p) => p.id === projectId)) return projectId;
    const head = [...projects].sort(sortByAttentionThenTime)[0];
    return head?.id ?? "";
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
      <div className="flex flex-1 items-center justify-center bg-surface text-sm text-ink-muted">
        Select a project to get started.
      </div>
    );
  }

  // Keep Conversations / Sessions / Board mounted (CSS hide) so tab switches
  // do not remount transcript panes or re-fetch tails (avoids content flash).
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface">
      <ProjectHeader projectId={resolvedProjectId} />

      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-1 overflow-hidden",
          projectView !== "conversations" && "hidden",
        )}
        // Inert when hidden so focus/shortcuts stay in the active view.
        inert={projectView !== "conversations" ? true : undefined}
        aria-hidden={projectView !== "conversations"}
      >
        {listCollapsed ? (
          <>
            <ConversationListRail />
            <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
              {conversationId ? (
                <Timeline
                  key={conversationId}
                  conversationId={conversationId}
                />
              ) : (
                <TimelineEmpty />
              )}
              {detailsOpen && conversationId ? (
                // Not inside a resizable Panel — fixed rail width (fill=false).
                <SessionInspector conversationId={conversationId} />
              ) : conversationId ? (
                <InspectorToggle onOpen={toggleDetails} />
              ) : null}
            </div>
          </>
        ) : (
          <Group
            orientation="horizontal"
            className="flex min-h-0 min-w-0 flex-1 overflow-hidden"
            defaultLayout={{
              list: 22,
              timeline: detailsOpen ? 53 : 78,
              inspector: detailsOpen ? 25 : 0,
            }}
          >
            <Panel
              id="list"
              minSize={180}
              defaultSize="22"
              className="min-h-0 min-w-0"
            >
              <ConversationList
                key={resolvedProjectId}
                projectId={resolvedProjectId}
                fill
              />
            </Panel>
            <Separator
              className={cn(
                "w-1.5 shrink-0 bg-transparent transition-colors duration-150",
                "hover:bg-accent/30 data-[separator-active]:bg-accent/40",
              )}
            />
            <Panel
              id="timeline"
              minSize={280}
              defaultSize={detailsOpen ? "53" : "78"}
              className="flex min-h-0 min-w-0 flex-col overflow-hidden"
            >
              {conversationId ? (
                <Timeline
                  key={conversationId}
                  conversationId={conversationId}
                />
              ) : (
                <TimelineEmpty />
              )}
            </Panel>
            {detailsOpen && conversationId ? (
              <>
                <Separator
                  className={cn(
                    "w-1.5 shrink-0 bg-transparent transition-colors duration-150",
                    "hover:bg-accent/30 data-[separator-active]:bg-accent/40",
                  )}
                />
                <Panel
                  id="inspector"
                  minSize={200}
                  defaultSize="25"
                  className="min-h-0 min-w-0"
                >
                  <SessionInspector conversationId={conversationId} fill />
                </Panel>
              </>
            ) : conversationId ? (
              <InspectorToggle onOpen={toggleDetails} />
            ) : null}
          </Group>
        )}
      </div>

      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-1 overflow-hidden",
          projectView !== "sessions" && "hidden",
        )}
        inert={projectView !== "sessions" ? true : undefined}
        aria-hidden={projectView !== "sessions"}
      >
        <ErrorBoundary label="sessions">
          <SessionsView key={resolvedProjectId} projectId={resolvedProjectId} />
        </ErrorBoundary>
      </div>

      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-1 overflow-hidden",
          projectView !== "board" && "hidden",
        )}
        inert={projectView !== "board" ? true : undefined}
        aria-hidden={projectView !== "board"}
      >
        <ProjectBoard key={resolvedProjectId} projectId={resolvedProjectId} />
      </div>
    </div>
  );
}

function InspectorToggle({ onOpen }: { onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      title="Show inspector"
      className="flex w-10 shrink-0 flex-col items-center border-l border-ink/5 bg-surface pt-3 text-ink-muted transition-colors duration-150 hover:bg-surface-hover hover:text-ink"
    >
      <PanelRight className="h-4 w-4" />
    </button>
  );
}
