import { Bot, Columns3, List, Plus, Search } from "lucide-react";
import { useUiStore, type ProjectView } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  projectHostLabel,
  projectHostPillClass,
} from "@/shared/lib/host-status";
import { cn } from "@/shared/lib/utils";

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
  const listStatus = useWorkspaceStore((s) =>
    resolvedProjectId
      ? s.conversationsStatusByProject[resolvedProjectId]
      : undefined,
  );

  const project = projects.find((p) => p.id === resolvedProjectId);
  const hostLabel = projectHostLabel(project?.hostName);
  // Prefer loaded list length; only fall back to project metadata while idle/loading
  // and the list has not arrived yet — never mask a successful empty list as "3".
  const loadedCount = conversations.filter(
    (c) => c.projectId === resolvedProjectId,
  ).length;
  const listPhase = listStatus?.phase ?? "idle";
  const convCount =
    listPhase === "ready" || loadedCount > 0
      ? loadedCount
      : (project?.conversationCount ?? 0);
  const needYou = project?.needsAttention ?? 0;

  if (!project) {
    return (
      <header className="shrink-0 border-b border-ink/5 bg-surface px-5 py-4 text-[13px] text-ink-muted">
        Select or create a project to get started.
      </header>
    );
  }

  return (
    <header className="shrink-0 border-b border-ink/5 bg-surface px-4 pt-3 sm:px-5">
      <div className="flex min-w-0 items-center gap-2 sm:gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-2 sm:gap-3">
          <h1
            className="max-w-[40%] shrink truncate text-[16px] font-semibold tracking-tight text-ink sm:max-w-[30%] sm:text-[17px]"
            title={project.name}
          >
            {project.name}
          </h1>
          <span className="hidden h-4 w-px shrink-0 bg-ink/10 sm:block" />
          <p
            className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-muted sm:text-[12px]"
            title={project.workspacePath}
          >
            {project.workspacePath}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5 sm:gap-2">
          <button
            type="button"
            className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-ink/10 bg-surface-muted/80 px-2.5 text-[12px] font-medium text-ink-secondary transition-colors hover:bg-surface-hover sm:px-3"
          >
            <Search className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">Search</span>
          </button>
          <button
            type="button"
            onClick={() => {
              void (async () => {
                const title = "New conversation";
                const id = await createConversation(
                  project.id,
                  title,
                );
                if (id) selectConversation(id);
              })();
            }}
            className="inline-flex h-9 items-center gap-1.5 rounded-lg bg-ink px-2.5 text-[12px] font-semibold text-white transition-opacity hover:opacity-90 sm:px-3"
          >
            <Plus className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">New conversation</span>
            <span className="sm:hidden">New</span>
          </button>
        </div>
      </div>

      <div className="mt-2.5 flex min-w-0 items-center gap-1">
        {views.map((view) => {
          const Icon = view.icon;
          const active = projectView === view.id;
          return (
            <button
              key={view.id}
              type="button"
              onClick={() => setProjectView(view.id)}
              className={cn(
                "relative inline-flex h-9 shrink-0 items-center gap-1.5 px-2.5 text-[13px] font-medium transition-colors sm:px-3",
                active ? "text-ink" : "text-ink-muted hover:text-ink-secondary",
              )}
            >
              <Icon className="h-3.5 w-3.5" />
              {view.label}
              {active ? (
                <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-ink" />
              ) : null}
            </button>
          );
        })}
        <div className="ml-auto flex min-w-0 items-center gap-2 pb-0.5 text-[11px] text-ink-muted sm:gap-3 sm:text-[12px]">
          <span className="hidden truncate tabular-nums sm:inline">
            <strong className="font-semibold text-ink-secondary">
              {convCount}
            </strong>{" "}
            conversations
          </span>
          {needYou > 0 ? (
            <span className="truncate">
              <strong className="font-semibold text-rose-600 tabular-nums">
                {needYou}
              </strong>{" "}
              need you
            </span>
          ) : null}
          <span
            className={cn(
              "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold tracking-wide",
              projectHostPillClass(hostLabel),
            )}
            title="Host that owns this project"
          >
            {hostLabel}
          </span>
        </div>
      </div>
    </header>
  );
}
