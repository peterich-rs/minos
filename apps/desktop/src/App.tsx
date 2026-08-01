import { useEffect, useLayoutEffect } from "react";
import { AppShell } from "@/app/AppShell";
import { BootScreen } from "@/app/BootScreen";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import { LoginPage } from "@/features/auth/LoginPage";
import { ErrorBoundary } from "@/shared/ui/ErrorBoundary";
import { decideDesktopRoot } from "@/shared/lib/desktop-root-gate";
import { emitInitialRenderReady } from "@/shared/lib/initial-render-ready";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { useAccountStore } from "@/store/account-store";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";

export default function App() {
  const hydrateAuth = useAccountStore((s) => s.hydrateAuth);
  const authPhase = useAccountStore((s) => s.authPhase);

  const bootstrap = useWorkspaceStore((s) => s.bootstrap);
  const booting = useWorkspaceStore((s) => s.booting);
  const bootPhase = useWorkspaceStore((s) => s.bootPhase);
  const bootProgress = useWorkspaceStore((s) => s.bootProgress);
  const projects = useWorkspaceStore((s) => s.projects);
  const projectId = useUiStore((s) => s.projectId);
  const selectProject = useUiStore((s) => s.selectProject);

  // Always mounted (boot + login + shell) so stored minos:text-scale applies
  // before AppShell, and Cmd± works during BootScreen / LoginPage.
  useWebviewZoomShortcuts();

  // First layout commit → host may show the window (BootScreen is fine).
  // Do not wait for bootstrap; that would delay reveal unnecessarily.
  useLayoutEffect(() => {
    emitInitialRenderReady();
  }, []);

  // Account gate first — no AppShell without a valid Minos session.
  useEffect(() => {
    void hydrateAuth();
  }, [hydrateAuth]);

  // Daemon bootstrap only after sign-in (or restored session).
  useEffect(() => {
    if (authPhase !== "authenticated") return;
    void bootstrap();
  }, [authPhase, bootstrap]);

  // After boot: restore last-used project (persisted by ui-store); if the id
  // is stale or missing, fall back to the sorted head (attention-first).
  // Conversation list + detail load are owned by WorkView / Timeline.
  useEffect(() => {
    if (authPhase !== "authenticated" || booting) return;
    if (projects.length === 0) {
      if (projectId) selectProject("");
      return;
    }
    if (projects.some((p) => p.id === projectId)) return;
    const head = [...projects].sort(sortByAttentionThenTime)[0];
    selectProject(head?.id ?? "");
  }, [authPhase, booting, projects, projectId, selectProject]);

  const surface = decideDesktopRoot({
    authPhase,
    workspaceBooting: authPhase === "authenticated" && booting,
  });

  if (surface === "boot") {
    // Full-viewport boot — no canvas margin / shell chrome (those peek as a
    // "background frame" behind loading).
    const phase =
      authPhase === "booting" ? "Checking account…" : bootPhase;
    const progress = authPhase === "booting" ? 12 : bootProgress;
    return (
      <div className="fixed inset-0 z-[100] h-full w-full">
        <BootScreen phase={phase} progress={progress} />
      </div>
    );
  }

  if (surface === "login") {
    return (
      <div className="fixed inset-0 z-[100] h-full w-full">
        <LoginPage />
      </div>
    );
  }

  return (
    <ErrorBoundary label="app">
      <AppShell />
    </ErrorBoundary>
  );
}
