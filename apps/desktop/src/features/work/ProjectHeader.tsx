import { useEffect, useState } from "react";
import { Bot, Columns3, List } from "lucide-react";
import { CreateConversationDialog } from "@/features/work/ui/CreateConversationDialog";
import type { CreateConversationFormInput } from "@/features/work/lib/create-conversation-form";
import { useUiStore, type ProjectView } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  projectHostLabel,
  projectHostPillClass,
} from "@/shared/lib/host-status";
import { cn } from "@/shared/lib/utils";
import { toast } from "@/shared/lib/toast";
import { useAgentProfilesQuery } from "@/shared/api/hooks";
import { WorkProjectHeader } from "@/shared/ui/WorkChrome";

const views: { id: ProjectView; label: string; icon: typeof List }[] = [
  { id: "conversations", label: "Conversations", icon: List },
  { id: "sessions", label: "Sessions", icon: Bot },
  { id: "board", label: "Board", icon: Columns3 },
];

export function ProjectHeader({ projectId }: { projectId?: string }) {
  const uiProjectId = useUiStore((s) => s.projectId);
  const resolvedProjectId = projectId || uiProjectId;
  const projectView = useUiStore((s) => s.projectView);
  const setProjectView = useUiStore((s) => s.setProjectView);
  const selectConversation = useUiStore((s) => s.selectConversation);
  const projects = useWorkspaceStore((s) => s.projects);
  const conversations = useWorkspaceStore((s) => s.conversations);
  const createConversation = useWorkspaceStore((s) => s.createConversation);
  const clis = useWorkspaceStore((s) => s.clis);
  const loadClis = useWorkspaceStore((s) => s.loadClis);
  const source = useWorkspaceStore((s) => s.source);
  const listStatus = useWorkspaceStore((s) =>
    resolvedProjectId
      ? s.conversationsStatusByProject[resolvedProjectId]
      : undefined,
  );
  const [createOpen, setCreateOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const profilesQuery = useAgentProfilesQuery();
  const profiles = profilesQuery.data ?? [];

  const project = projects.find((p) => p.id === resolvedProjectId);
  const hostLabel = projectHostLabel(project?.hostName);
  const loadedCount = conversations.filter(
    (c) => c.projectId === resolvedProjectId,
  ).length;
  const listPhase = listStatus?.phase ?? "idle";
  const convCount =
    listPhase === "ready" || loadedCount > 0
      ? loadedCount
      : (project?.conversationCount ?? 0);
  const needYou = project?.needsAttention ?? 0;

  useEffect(() => {
    if (source !== "daemon") return;
    void loadClis({ quiet: true });
  }, [source, loadClis]);

  const handleCreate = async (input: CreateConversationFormInput) => {
    if (!project) return;
    setCreating(true);
    try {
      const id = await createConversation(project.id, input);
      if (id) selectConversation(id);
      const actionError = useWorkspaceStore.getState().actionError;
      if (actionError) {
        toast.error(actionError);
      }
    } finally {
      setCreating(false);
    }
  };

  if (!project) {
    return (
      <header className="shrink-0 border-b border-ink/6 bg-surface px-5 py-4 text-sm text-ink-muted">
        Select or create a project to get started.
      </header>
    );
  }

  return (
    <>
      <WorkProjectHeader
        projectName={project.name}
        projectPath={project.workspacePath}
        tabs={views.map((v) => {
          const Icon = v.icon;
          return {
            id: v.id,
            label: v.label,
            icon: <Icon className="h-3.5 w-3.5" />,
          };
        })}
        activeTabId={projectView}
        onTabSelect={(id) => setProjectView(id as ProjectView)}
        onNew={() => setCreateOpen(true)}
        newDisabled={creating}
        meta={
          <>
            <span className="hidden truncate tabular-nums sm:inline">
              <strong className="font-semibold text-ink-secondary">
                {convCount}
              </strong>{" "}
              conversations
            </span>
            {needYou > 0 ? (
              <span className="truncate">
                <strong className="font-semibold text-status-approval tabular-nums">
                  {needYou}
                </strong>{" "}
                need you
              </span>
            ) : null}
            <span
              className={cn(
                "shrink-0 rounded-full px-2 py-0.5 text-3xs font-semibold tracking-wide",
                projectHostPillClass(hostLabel),
              )}
              title="Host that owns this project"
            >
              {hostLabel}
            </span>
          </>
        }
      />

      <CreateConversationDialog
        open={createOpen}
        isCreating={creating}
        projectName={project.name}
        clis={clis}
        profiles={profiles}
        onOpenChange={setCreateOpen}
        onCreate={handleCreate}
      />
    </>
  );
}
