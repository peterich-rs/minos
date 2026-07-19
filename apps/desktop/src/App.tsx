import { useEffect } from "react";
import { AppShell } from "@/components/shell/AppShell";
import { BootScreen } from "@/components/shell/BootScreen";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useUiStore } from "@/store/ui-store";

export default function App() {
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);
  const booting = useWorkspaceStore((s) => s.booting);
  const bootPhase = useWorkspaceStore((s) => s.bootPhase);
  const bootProgress = useWorkspaceStore((s) => s.bootProgress);
  const projects = useWorkspaceStore((s) => s.projects);
  const projectId = useUiStore((s) => s.projectId);
  const selectProject = useUiStore((s) => s.selectProject);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  // After boot: select first project if none selected / stale id.
  // Conversation list + detail load are owned by WorkView / Timeline.
  useEffect(() => {
    if (booting) return;
    if (projects.length === 0) {
      if (projectId) selectProject("");
      return;
    }
    if (!projects.some((p) => p.id === projectId)) {
      selectProject(projects[0]!.id);
    }
  }, [booting, projects, projectId, selectProject]);

  if (booting) {
    return (
      <div className="h-full w-full bg-canvas p-3 sm:p-4">
        <div className="h-full w-full overflow-hidden rounded-shell border border-white/60 bg-surface shadow-shell">
          <BootScreen phase={bootPhase} progress={bootProgress} />
        </div>
      </div>
    );
  }

  return (
    <ErrorBoundary label="app">
      <AppShell />
    </ErrorBoundary>
  );
}
