import { useEffect } from "react";
import { Sidebar } from "./Sidebar";
import { WorkView } from "@/features/work/WorkView";
import { AttentionView } from "@/features/attention/AttentionView";
import { AgentsView } from "@/features/agents/AgentsView";
import { HostView } from "@/features/host/HostView";
import { CommandPalette } from "./CommandPalette";
import { ConnectionToasts } from "./ConnectionToasts";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { ShellFrame } from "@/shared/layout/ShellFrame";
import { Toaster } from "@/shared/ui/toaster";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { useUiStore } from "@/store/ui-store";

export function AppShell() {
  const primaryNav = useUiStore((s) => s.primaryNav);
  const cmdOpen = useUiStore((s) => s.commandPaletteOpen);
  const setCmdOpen = useUiStore((s) => s.setCommandPaletteOpen);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (hasPrimaryShortcutModifier(e) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCmdOpen(!useUiStore.getState().commandPaletteOpen);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setCmdOpen]);

  return (
    <TooltipProvider delayDuration={280} skipDelayDuration={120}>
      <ShellFrame sidebar={<Sidebar />}>
        <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {primaryNav === "work" ? <WorkView /> : null}
          {primaryNav === "attention" ? <AttentionView /> : null}
          {primaryNav === "agents" ? <AgentsView /> : null}
          {primaryNav === "host" ? <HostView /> : null}
        </main>
        <CommandPalette open={cmdOpen} onOpenChange={setCmdOpen} />
        <ConnectionToasts />
        <Toaster />
      </ShellFrame>
    </TooltipProvider>
  );
}
