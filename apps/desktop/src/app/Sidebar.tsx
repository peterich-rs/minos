import { useEffect, useState } from "react";
import {
  FolderGit2,
  LayoutDashboard,
  Bot,
  Monitor,
  AlertTriangle,
  Circle,
} from "lucide-react";
import { CreateProjectEmpty } from "@/features/work/CreateProjectEmpty";
import { SidebarUpdateCard } from "@/features/settings/SidebarUpdateCard";
import { useUiStore, type PrimaryNav } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useAccountStore } from "@/store/account-store";
import {
  deriveHostPresence,
  presenceDotClass,
  projectHostLabel,
} from "@/shared/lib/host-status";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { cn } from "@/shared/lib/utils";
import {
  AppRail,
  AppRailProjectRow,
  AppRailProjectsHeader,
} from "@/shared/ui/AppRail";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { SidebarConnectionCard } from "@/app/SidebarConnectionCard";

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
  const refreshDaemonStatus = useWorkspaceStore((s) => s.refreshDaemonStatus);
  const cloudStatus = useAccountStore((s) => s.cloudStatus);
  const accountSyncStatus = useAccountStore((s) => s.accountSyncStatus);
  const session = useAccountStore((s) => s.session);
  const syncCloudFromHub = useAccountStore((s) => s.syncCloudFromHub);
  const [updateDismissed, setUpdateDismissed] = useState(false);
  const attention = projects.reduce((sum, p) => sum + p.needsAttention, 0);

  // Keep Host cloudOnline fresh (secondary readiness for bot runtime).
  useEffect(() => {
    if (source !== "daemon") return;
    void refreshDaemonStatus();
    const id = window.setInterval(() => {
      void refreshDaemonStatus();
    }, 8000);
    return () => window.clearInterval(id);
  }, [source, refreshDaemonStatus]);

  useEffect(() => {
    syncCloudFromHub(connection?.cloudOnline);
  }, [connection?.cloudOnline, syncCloudFromHub]);

  // Primary Online = Account IM sync; Host is secondary (bot runtime).
  const presence = deriveHostPresence({
    source,
    daemonConnected: source === "daemon" && connection?.connected === true,
    accountSync: session ? accountSyncStatus : "unknown",
    cloud: session ? cloudStatus : "unknown",
    cloudOnline: connection?.cloudOnline,
  });
  const presenceTitle =
    presence.cloud === "online" && !presence.hostReady
      ? "Account online · Host offline — bots unavailable"
      : presence.cloud === "online" && presence.hostReady
        ? "Account online · Host ready"
        : "Open Host status";

  return (
    <AppRail
      brandSubtitle={
        <button
          type="button"
          onClick={() => setPrimaryNav("host")}
          title={presenceTitle}
          className="flex max-w-full items-center gap-1 rounded-md text-left transition-colors duration-150 hover:text-ink"
        >
          <Circle
            className={cn(
              "h-2 w-2 shrink-0 fill-current",
              presenceDotClass(presence.tone),
            )}
          />
          <span className="truncate">
            {presence.label}
            {presence.cloud === "online" && !presence.hostReady
              ? " · bots offline"
              : ""}
          </span>
        </button>
      }
      navItems={navItems.map((item) => {
        const Icon = item.icon;
        return {
          id: item.id,
          label: item.label,
          badge: item.id === "attention" ? attention : 0,
          icon: <Icon strokeWidth={1.8} />,
        };
      })}
      activeNavId={primaryNav}
      onNavSelect={(id) => setPrimaryNav(id as PrimaryNav)}
      projectsHeader={
        <AppRailProjectsHeader
          action={<CreateProjectEmpty variant="inline" />}
        />
      }
      projects={
        <>
          {[...projects].sort(sortByAttentionThenTime).map((project) => {
            const active = primaryNav === "work" && project.id === projectId;
            return (
              <AppRailProjectRow
                key={project.id}
                name={project.name}
                path={project.workspacePath}
                active={active}
                attention={project.needsAttention ?? 0}
                running={project.runningAgents > 0}
                hostLabel={
                  project.hostName
                    ? projectHostLabel(project.hostName)
                    : null
                }
                onClick={() => selectProject(project.id)}
                leading={
                  <FolderGit2
                    className={cn(
                      "mt-0.5 h-4 w-4 shrink-0",
                      active ? "text-primary" : "text-ink-muted",
                    )}
                    strokeWidth={1.8}
                  />
                }
              />
            );
          })}
          {projects.length === 0 ? (
            <p className="px-2 py-3 text-xs text-ink-muted">
              No projects yet — use the big + on the right.
            </p>
          ) : null}
        </>
      }
      footer={
        <div className="shrink-0">
          <SidebarConnectionCard />
          {!updateDismissed ? (
            <SidebarUpdateCard onDismiss={() => setUpdateDismissed(true)} />
          ) : null}
          <div className="border-t border-ink/8 px-3 py-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => setCommandPaletteOpen(true)}
                  className="flex w-full items-center justify-between rounded-lg px-2 py-1.5 text-2xs text-ink-muted transition-colors duration-150 hover:bg-ink/5 hover:text-ink-secondary"
                >
                  <span>Command palette</span>
                  <kbd className="rounded border border-ink/10 bg-surface/70 px-1.5 py-0.5 font-mono text-3xs backdrop-blur-sm">
                    ⌘K
                  </kbd>
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">Jump anywhere (⌘K)</TooltipContent>
            </Tooltip>
          </div>
        </div>
      }
    />
  );
}
