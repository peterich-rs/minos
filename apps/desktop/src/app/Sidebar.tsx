import { motion, useReducedMotion } from "motion/react";
import {
  FolderGit2,
  LayoutDashboard,
  Bot,
  Monitor,
  AlertTriangle,
  Sparkles,
  Circle,
} from "lucide-react";
import { CreateProjectEmpty } from "@/features/work/CreateProjectEmpty";
import { useUiStore, type PrimaryNav } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  deriveHostPresence,
  presenceDotClass,
  projectHostLabel,
} from "@/shared/lib/host-status";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { cn } from "@/shared/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

const navItems: {
  id: PrimaryNav;
  label: string;
  icon: typeof LayoutDashboard;
}[] = [
  { id: "work", label: "Work", icon: LayoutDashboard },
  { id: "attention", label: "Attention", icon: AlertTriangle },
  { id: "agents", label: "Agents", icon: Bot },
  { id: "host", label: "Host", icon: Monitor },
];

export function Sidebar() {
  const primaryNav = useUiStore((s) => s.primaryNav);
  const setPrimaryNav = useUiStore((s) => s.setPrimaryNav);
  const setCommandPaletteOpen = useUiStore((s) => s.setCommandPaletteOpen);
  const projectId = useUiStore((s) => s.projectId);
  const selectProject = useUiStore((s) => s.selectProject);
  const projects = useWorkspaceStore((s) => s.projects);
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const reduceMotion = useReducedMotion();
  const attention = projects.reduce((sum, p) => sum + p.needsAttention, 0);
  // v1: only local daemon is wired; relayLinked stays false → "Local only".
  const presence = deriveHostPresence({
    source,
    daemonConnected: source === "daemon" && connection?.connected === true,
    relayLinked: false,
  });

  return (
    <aside className="flex w-[240px] shrink-0 flex-col border-r border-ink/5 bg-surface">
      <div className="flex items-center gap-2.5 border-b border-ink/5 px-4 py-3.5">
        <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-ink text-white">
          <Sparkles className="h-4 w-4" strokeWidth={2.2} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold tracking-tight text-ink">
            Minos
          </div>
          <button
            type="button"
            onClick={() => setPrimaryNav("host")}
            title="Open Host status"
            className="mt-0.5 flex max-w-full items-center gap-1 rounded-md text-left text-2xs text-ink-muted transition-colors duration-150 hover:text-ink-secondary"
          >
            <Circle
              className={cn(
                "h-2 w-2 shrink-0 fill-current",
                presenceDotClass(presence.tone),
              )}
            />
            <span className="truncate">{presence.label}</span>
          </button>
        </div>
      </div>

      <nav className="space-y-0.5 px-2 py-3">
        {navItems.map((item) => {
          const Icon = item.icon;
          const active = primaryNav === item.id;
          const badge = item.id === "attention" ? attention : 0;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => setPrimaryNav(item.id)}
              className={cn(
                "relative flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors duration-150",
                active
                  ? "font-medium text-ink"
                  : "text-ink-secondary hover:bg-surface-hover",
              )}
            >
              {active ? (
                <motion.span
                  layoutId="sidebar-nav-pill"
                  className="absolute inset-0 rounded-lg bg-surface-muted"
                  transition={
                    reduceMotion
                      ? { duration: 0 }
                      : { type: "spring", stiffness: 420, damping: 36 }
                  }
                />
              ) : null}
              <Icon
                className="relative z-[1] h-4 w-4 shrink-0 opacity-80"
                strokeWidth={1.8}
              />
              <span className="relative z-[1] flex-1">{item.label}</span>
              {badge > 0 ? (
                <span className="relative z-[1] rounded-full bg-rose-500 px-1.5 py-0.5 text-3xs font-semibold text-white">
                  {badge}
                </span>
              ) : null}
            </button>
          );
        })}
      </nav>

      <div className="mt-1 flex items-center justify-between px-4 pb-1.5 pt-2">
        <span className="text-2xs font-semibold uppercase tracking-[0.06em] text-ink-muted">
          Projects
        </span>
        <CreateProjectEmpty variant="inline" />
      </div>

      <div className="scrollbar-thin flex-1 space-y-0.5 overflow-y-auto px-2 pb-3">
        {[...projects].sort(sortByAttentionThenTime).map((project) => {
          const active = primaryNav === "work" && project.id === projectId;
          return (
            <button
              key={project.id}
              type="button"
              onClick={() => selectProject(project.id)}
              className={cn(
                "flex w-full items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors duration-150",
                active
                  ? "bg-accent-soft ring-1 ring-accent/30"
                  : "hover:bg-surface-hover",
              )}
            >
              <FolderGit2
                className={cn(
                  "mt-0.5 h-4 w-4 shrink-0",
                  active ? "text-accent-strong" : "text-ink-muted",
                )}
                strokeWidth={1.8}
              />
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-1.5">
                  <span
                    className="min-w-0 flex-1 truncate text-sm font-medium text-ink"
                    title={project.name}
                  >
                    {project.name}
                  </span>
                  {project.runningAgents > 0 ? (
                    <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />
                  ) : null}
                  {project.needsAttention > 0 ? (
                    <span className="ml-auto shrink-0 rounded-full bg-rose-500/90 px-1.5 text-3xs font-semibold text-white">
                      {project.needsAttention}
                    </span>
                  ) : null}
                </div>
                <div
                  className="truncate text-2xs text-ink-muted"
                  title={project.workspacePath}
                >
                  {project.workspacePath.replace(/^~\//, "")}
                </div>
                {project.hostName ? (
                  <div className="mt-0.5 truncate text-3xs text-ink-muted/90">
                    {projectHostLabel(project.hostName)}
                  </div>
                ) : null}
              </div>
            </button>
          );
        })}
        {projects.length === 0 ? (
          <p className="px-2 py-3 text-xs text-ink-muted">
            No projects yet — use the big + on the right.
          </p>
        ) : null}
      </div>

      <div className="border-t border-ink/5 px-3 py-2">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={() => setCommandPaletteOpen(true)}
              className="flex w-full items-center justify-between rounded-lg px-2 py-1.5 text-2xs text-ink-muted transition-colors duration-150 hover:bg-surface-hover hover:text-ink-secondary"
            >
              <span>Command palette</span>
              <kbd className="rounded border border-ink/10 bg-surface-muted px-1.5 py-0.5 font-mono text-3xs">
                ⌘K
              </kbd>
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">Jump anywhere (⌘K)</TooltipContent>
        </Tooltip>
      </div>
    </aside>
  );
}
