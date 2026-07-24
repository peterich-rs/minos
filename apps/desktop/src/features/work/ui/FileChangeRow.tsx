import { displayPath, type FileChangeEntry } from "@/shared/lib/session-summary";
import { cn } from "@/shared/lib/utils";

export function FileChangeRow({ file }: { file: FileChangeEntry }) {
  const short = displayPath(file.path);
  return (
    <li
      className={cn(
        "rounded-lg px-2 py-1.5 font-mono text-[11px] leading-snug",
        file.failed
          ? "bg-rose-50/80 text-rose-900"
          : "bg-surface-muted/50 text-ink-secondary",
      )}
      title={file.path}
    >
      <div className="break-all text-ink">{short}</div>
      <div className="mt-0.5 flex items-center gap-2 tabular-nums">
        {file.del > 0 || file.add > 0 ? (
          <>
            <span className="text-rose-700">-{file.del}</span>
            <span className="text-emerald-700">+{file.add}</span>
          </>
        ) : (
          <span className="text-ink-muted">
            {file.failed ? "failed" : file.ok ? "touched" : "pending…"}
          </span>
        )}
      </div>
    </li>
  );
}
