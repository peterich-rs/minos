import { useState } from "react";
import { FolderOpen, Plus } from "lucide-react";
import { pickWorkspaceFolder } from "@/shared/lib/pick-folder";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";

type Props = {
  /** Compact mode for sidebar + button; full canvas for main empty state. */
  variant?: "full" | "inline";
  className?: string;
};

/**
 * Primary entry to create a project by selecting a workspace folder.
 * Wired to daemon `minos_local_create_project` via workspace-store.
 */
export function CreateProjectEmpty({ variant = "full", className }: Props) {
  const createProject = useWorkspaceStore((s) => s.createProject);
  const loading = useWorkspaceStore((s) => s.loading);
  const error = useWorkspaceStore((s) => s.error);
  const selectProject = useUiStore((s) => s.selectProject);
  const setPrimaryNav = useUiStore((s) => s.setPrimaryNav);
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const onCreate = async () => {
    setLocalError(null);
    setBusy(true);
    try {
      const path = await pickWorkspaceFolder();
      if (!path) {
        setBusy(false);
        return;
      }
      const id = await createProject(path);
      setPrimaryNav("work");
      selectProject(id);
    } catch (e) {
      setLocalError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const pending = busy || loading;
  const message = localError || error;

  if (variant === "inline") {
    return (
      <button
        type="button"
        disabled={pending}
        onClick={() => void onCreate()}
        title="New project"
        className={cn(
          "rounded-md p-0.5 text-ink-muted hover:bg-surface-hover hover:text-ink disabled:opacity-40",
          className,
        )}
      >
        <Plus className="h-3.5 w-3.5" />
      </button>
    );
  }

  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center bg-surface px-8",
        className,
      )}
    >
      <button
        type="button"
        disabled={pending}
        onClick={() => void onCreate()}
        className={cn(
          "group flex w-full max-w-md flex-col items-center gap-5 rounded-3xl border border-dashed border-ink/15 bg-surface-muted/40 px-10 py-14 text-center transition-all",
          "hover:border-ink/25 hover:bg-surface-muted/70 hover:shadow-sm",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ink/20",
          pending && "pointer-events-none opacity-60",
        )}
      >
        <span className="flex h-20 w-20 items-center justify-center rounded-2xl bg-ink text-surface shadow-md transition-transform group-hover:scale-105">
          {pending ? (
            <span className="h-8 w-8 animate-pulse rounded-full border-2 border-white/40 border-t-white" />
          ) : (
            <Plus className="h-10 w-10" strokeWidth={2} />
          )}
        </span>
        <div className="space-y-2">
          <h2 className="text-lg font-semibold tracking-tight text-ink">
            Create your first project
          </h2>
          <p className="text-sm leading-relaxed text-ink-muted">
            Choose a local folder as the workspace. Minos will create a project
            on the daemon and open it here.
          </p>
        </div>
        <span className="inline-flex items-center gap-2 rounded-full bg-surface px-3 py-1.5 text-xs font-medium text-ink-secondary ring-1 ring-ink/10">
          <FolderOpen className="h-3.5 w-3.5" />
          Select folder
        </span>
      </button>
      {message ? (
        <p className="mt-4 max-w-md text-center text-xs text-rose-600">
          {message}
        </p>
      ) : null}
    </div>
  );
}
