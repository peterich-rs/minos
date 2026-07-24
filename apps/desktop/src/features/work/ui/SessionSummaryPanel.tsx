import { FileDiff, Loader2 } from "lucide-react";
import { StatusPill } from "@/shared/ui/StatusPill";
import type { ProjectSession } from "@/store/workspace-store";
import { type summarizeSessionFromTranscript } from "@/shared/lib/session-summary";
import { FileChangeRow } from "./FileChangeRow";

export function SessionSummaryPanel({
  session,
  summary,
}: {
  session: ProjectSession;
  summary: ReturnType<typeof summarizeSessionFromTranscript>;
}) {
  return (
    <aside className="flex min-h-0 w-[min(280px,32vw)] min-w-[220px] max-w-[320px] shrink-0 flex-col self-stretch overflow-hidden border-l border-ink/5 bg-surface">
      <div className="flex shrink-0 items-center gap-2 border-b border-ink/5 px-3 py-2.5">
        <FileDiff className="h-3.5 w-3.5 text-ink-muted" />
        <div className="min-w-0 flex-1">
          <div className="text-xs font-semibold text-ink">Summary</div>
          <div className="text-3xs text-ink-muted">
            Session stats from tools
          </div>
        </div>
      </div>

      <div
        className="scrollbar-thin min-h-0 flex-1 space-y-4 overflow-y-auto overscroll-contain px-3 py-3"
        style={{ flex: "1 1 0%" }}
      >
        <section>
          <div className="mb-1.5 text-3xs font-semibold uppercase tracking-wide text-ink-muted">
            Status
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <StatusPill status={session.status} />
            {summary.pendingEdits > 0 &&
            (session.status === "running" ||
              session.status === "needs_approval") ? (
              <span className="inline-flex items-center gap-1 text-2xs text-amber-800">
                <Loader2 className="h-3 w-3 animate-spin" />
                {summary.pendingEdits} edit
                {summary.pendingEdits === 1 ? "" : "s"} in flight
              </span>
            ) : null}
          </div>
        </section>

        <section>
          <div className="mb-1.5 text-3xs font-semibold uppercase tracking-wide text-ink-muted">
            Activity
          </div>
          <dl className="grid grid-cols-2 gap-x-2 gap-y-1.5 text-xs">
            <dt className="text-ink-muted">Tools</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.toolCallCount}
            </dd>
            <dt className="text-ink-muted">Edits</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.editCallCount}
            </dd>
            <dt className="text-ink-muted">Files</dt>
            <dd className="text-right font-medium tabular-nums text-ink">
              {summary.files.length}
            </dd>
            <dt className="text-ink-muted">Lines</dt>
            <dd className="text-right font-mono text-2xs tabular-nums">
              {summary.totalDel > 0 || summary.totalAdd > 0 ? (
                <>
                  <span className="text-rose-700">-{summary.totalDel}</span>
                  <span className="text-ink-muted"> / </span>
                  <span className="text-emerald-700">+{summary.totalAdd}</span>
                </>
              ) : (
                <span className="text-ink-muted">—</span>
              )}
            </dd>
          </dl>
          <p className="mt-2 text-3xs leading-snug text-ink-muted">
            Token usage is not shown yet (CLI formats differ; not in unified
            projection).
          </p>
        </section>

        <section>
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-3xs font-semibold uppercase tracking-wide text-ink-muted">
              Files changed
            </span>
            {summary.files.length > 0 ? (
              <span className="text-3xs tabular-nums text-ink-muted">
                {summary.files.length}
              </span>
            ) : null}
          </div>
          {summary.files.length === 0 ? (
            <p className="rounded-lg bg-surface-muted/60 px-2.5 py-3 text-2xs leading-snug text-ink-muted">
              No file edits in this transcript yet. Edit tools
              (write / search_replace / apply_patch …) appear here with{" "}
              <span className="font-mono">-N +M</span> when available.
            </p>
          ) : (
            <ul className="space-y-1">
              {summary.files.map((file) => (
                <FileChangeRow key={file.path} file={file} />
              ))}
            </ul>
          )}
        </section>
      </div>
    </aside>
  );
}
