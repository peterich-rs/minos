import { useEffect } from "react";
import { AppShell } from "@/components/shell/AppShell";
import { BootScreen } from "@/components/shell/BootScreen";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useUiStore } from "@/store/ui-store";
import { sortByAttentionThenTime } from "@/lib/list-sort";

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

  // After boot: restore last-used project (persisted by ui-store); if the id
  // is stale or missing, fall back to the sorted head (attention-first).
  // Conversation list + detail load are owned by WorkView / Timeline.
  useEffect(() => {
    if (booting) return;
    if (projects.length === 0) {
      if (projectId) selectProject("");
      return;
    }
    if (projects.some((p) => p.id === projectId)) return;
    const head = [...projects].sort(sortByAttentionThenTime)[0];
    selectProject(head?.id ?? "");
  }, [booting, projects, projectId, selectProject]);

  if (booting) {
    // Full-viewport boot — no canvas margin / shell chrome (those peek as a
    // "background frame" behind loading).
    return (
      <div className="fixed inset-0 z-[100] h-full w-full bg-surface">
        <BootScreen phase={bootPhase} progress={bootProgress} />
      </div>
    );
  }

  return (
    <ErrorBoundary label="app">
      <AppShell />
    </ErrorBoundary>
  );
}
