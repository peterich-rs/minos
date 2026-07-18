import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/lib/utils";

export function HostView() {
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const error = useWorkspaceStore((s) => s.error);
  const actionError = useWorkspaceStore((s) => s.actionError);
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);

  const connected = source === "daemon" && connection?.connected;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="border-b border-ink/5 px-6 py-5">
        <h1 className="text-[20px] font-semibold tracking-tight text-ink">
          Host
        </h1>
        <p className="mt-1 text-[13px] text-ink-muted">
          Local daemon bridge (same discovery file as TUI:{" "}
          <code className="rounded bg-surface-muted px-1 text-[12px]">
            ~/.minos/run/tui-daemon-rpc.json
          </code>
          ).
        </p>
      </header>
      <div className="grid max-w-2xl gap-3 p-6 sm:grid-cols-2">
        <Stat
          label="Data source"
          value={source}
          accent={connected ? "ok" : "warn"}
        />
        <Stat
          label="Daemon"
          value={
            connected
              ? connection?.managed
                ? "managed (in-process)"
                : "external (discovery)"
              : "offline / mock"
          }
          accent={connected ? "ok" : "warn"}
        />
        <Stat label="Endpoint" value={connection?.endpoint ?? "—"} />
        <Stat
          label="Connect mode"
          value={connection?.source ?? "—"}
        />
        <Stat
          label="Last error"
          value={connection?.error || error || actionError || "none"}
        />
      </div>
      <div className="px-6 pb-6">
        <button
          type="button"
          onClick={() => void bootstrap()}
          className="rounded-xl bg-ink px-4 py-2 text-[13px] font-semibold text-white hover:opacity-90"
        >
          Reconnect daemon
        </button>
      </div>
      <div className="mx-6 mb-6 rounded-2xl border border-dashed border-ink/15 bg-surface-muted/50 px-5 py-8 text-center">
        <div className="text-[13px] font-medium text-ink">Pairing QR</div>
        <p className="mt-1 text-[12px] text-ink-muted">
          Placeholder for host pairing flow. Local coding does not require it.
        </p>
        <div className="mx-auto mt-4 h-36 w-36 rounded-xl bg-surface ring-1 ring-ink/10" />
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: "ok" | "warn";
}) {
  return (
    <div className="rounded-2xl border border-ink/5 bg-surface p-4 shadow-sm">
      <div className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-muted">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 break-all text-[14px] font-semibold capitalize text-ink",
          accent === "ok" && "text-emerald-700",
          accent === "warn" && "text-amber-700",
        )}
      >
        {value}
      </div>
    </div>
  );
}
