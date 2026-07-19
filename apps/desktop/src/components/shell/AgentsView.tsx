import { useEffect } from "react";
import { CheckCircle2, CircleDashed } from "lucide-react";
import { agentMeta, type AgentRuntime } from "@/lib/mock-data";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/lib/utils";

export function AgentsView() {
  const clis = useWorkspaceStore((s) => s.clis);
  const clisStatus = useWorkspaceStore((s) => s.clisStatus);
  const loadClis = useWorkspaceStore((s) => s.loadClis);
  const source = useWorkspaceStore((s) => s.source);

  useEffect(() => {
    if (source !== "daemon") return;
    void loadClis();
  }, [source, loadClis]);

  const phase = clisStatus.phase;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="border-b border-ink/5 px-6 py-5">
        <h1 className="text-[20px] font-semibold tracking-tight text-ink">
          Agents
        </h1>
        <p className="mt-1 max-w-xl text-[13px] text-ink-muted">
          Local CLI runtimes detected on this Host. Chat always happens inside a
          Project conversation — this page is inventory only.
        </p>
      </header>
      {phase === "error" ? (
        <div className="flex flex-col items-center gap-3 px-6 py-10 text-center">
          <p className="text-[13px] text-rose-600">
            {clisStatus.error ?? "Failed to detect CLIs"}
          </p>
          <button
            type="button"
            onClick={() => void loadClis()}
            className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white"
          >
            Retry detect
          </button>
        </div>
      ) : null}
      <div className="scrollbar-thin grid flex-1 content-start gap-3 overflow-y-auto p-4 sm:grid-cols-2 lg:grid-cols-3">
        {phase === "loading" && clis.length === 0 ? (
          <p className="col-span-full py-12 text-center text-[13px] text-ink-muted">
            Detecting installed CLIs…
          </p>
        ) : null}
        {clis.map((rt) => {
          const agent = rt.agent as AgentRuntime;
          const meta = agentMeta[agent] ?? {
            label: rt.agent,
            color: "bg-stone-100 text-stone-700",
          };
          return (
            <div
              key={rt.agent}
              className="rounded-2xl border border-ink/5 bg-white p-4 shadow-sm"
            >
              <div className="flex items-start justify-between gap-2">
                <div>
                  <div
                    className={cn(
                      "inline-flex rounded-md px-2 py-0.5 text-[12px] font-semibold",
                      meta.color,
                    )}
                  >
                    {meta.label}
                  </div>
                  <div className="mt-2 text-[13px] text-ink-secondary">
                    @{rt.agent}
                  </div>
                </div>
                {rt.installed ? (
                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-700">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    Installed
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-ink-muted">
                    <CircleDashed className="h-3.5 w-3.5" />
                    Missing
                  </span>
                )}
              </div>
              {rt.installed ? (
                <dl className="mt-4 space-y-1.5 text-[12px]">
                  <div className="flex justify-between gap-2">
                    <dt className="text-ink-muted">Status</dt>
                    <dd className="font-medium text-ink">{rt.status}</dd>
                  </div>
                </dl>
              ) : (
                <p className="mt-4 text-[12px] text-ink-muted">
                  Install the CLI and re-detect to use @{rt.agent} in
                  conversations.
                </p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
