import { Sidebar } from "./Sidebar";
import { WorkView } from "./WorkView";
import { AttentionView } from "./AttentionView";
import { AgentsView } from "./AgentsView";
import { HostView } from "./HostView";
import { useUiStore } from "@/store/ui-store";

export function AppShell() {
  const primaryNav = useUiStore((s) => s.primaryNav);

  return (
    <div className="flex h-full w-full min-h-0 bg-surface">
      <div className="flex h-full w-full min-h-0 min-w-0 overflow-hidden">
        <Sidebar />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {primaryNav === "work" ? <WorkView /> : null}
          {primaryNav === "attention" ? <AttentionView /> : null}
          {primaryNav === "agents" ? <AgentsView /> : null}
          {primaryNav === "host" ? <HostView /> : null}
        </main>
      </div>
    </div>
  );
}
