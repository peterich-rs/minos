import { useEffect, useLayoutEffect } from "react";
import { AppShell } from "@/app/AppShell";
import { BootScreen } from "@/app/BootScreen";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import { ErrorBoundary } from "@/shared/ui/ErrorBoundary";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useUiStore } from "@/store/ui-store";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { emitInitialRenderReady } from "@/shared/lib/initial-render-ready";

export default function App() {
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);
  const booting = useWorkspaceStore((s) => s.booting);
  const bootPhase = useWorkspaceStore((s) => s.bootPhase);
  const bootProgress = useWorkspaceStore((s) => s.bootProgress);
  const projects = useWorkspaceStore((s) => s.projects);
  const projectId = useUiStore((s) => s.projectId);
  const selectProject = useUiStore((s) => s.selectProject);

  // Always mounted (boot + shell) so stored minos:text-scale applies before
  // AppShell, and Cmd± works during BootScreen.
  useWebviewZoomShortcuts();

  // First layout commit → host may show the window (BootScreen is fine).
  // Do not wait for bootstrap; that would delay reveal unnecessarily.
  useLayoutEffect(() => {
    emitInitialRenderReady();
  }, []);

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
