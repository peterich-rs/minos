import { useMemo } from "react";
import { cn } from "@/shared/lib/utils";
import {
  countDiffLines,
  isDiffLike,
  parseDiffLines,
  type DiffLineKind,
} from "@/shared/lib/diff-view";

const KIND_ROW: Record<
  DiffLineKind,
  { row: string; gutter: string; text: string; prefix?: string }
> = {
  add: {
    row: "bg-emerald-500/[0.08]",
    gutter: "text-emerald-700/70",
    text: "text-emerald-900",
    prefix: "+",
  },
  del: {
    row: "bg-rose-500/[0.08]",
    gutter: "text-rose-700/70",
    text: "text-rose-900",
    prefix: "-",
  },
  hunk: {
    row: "bg-sky-500/[0.06]",
    gutter: "text-sky-700/60",
    text: "text-sky-800/90",
  },
  meta: {
    row: "",
    gutter: "text-ink-muted/50",
    text: "text-ink-muted",
  },
  file: {
    row: "bg-ink/[0.03]",
    gutter: "text-ink-muted/60",
    text: "font-medium text-ink-secondary",
  },
  context: {
    row: "",
    gutter: "text-ink-muted/40",
    text: "text-ink-secondary",
  },
  ellipsis: {
    row: "bg-surface-muted/40",
    gutter: "text-ink-muted/50",
    text: "italic text-ink-muted",
  },
};

/**
 * Colored patch body for expanded Edit tools.
 * Keeps the same outer footprint as the old <pre max-h-72 overflow-auto>:
 * one nested scroll region (like before), no negative margins / layout hacks.
 */
export function DiffView({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  const body = typeof text === "string" ? text : "";
  const asDiff = isDiffLike(body);
  const lines = useMemo(
    () => (asDiff ? parseDiffLines(body, { head: 120, tail: 40 }) : []),
    [asDiff, body],
  );
  const stats = useMemo(
    () => (asDiff ? countDiffLines(body) : null),
    [asDiff, body],
  );

  if (!body.trim()) return null;

  // Fallback: identical to pre-diff transcript body.
  if (!asDiff) {
    return (
      <pre
        className={cn(
          "mt-1 max-h-72 overflow-auto rounded-lg border border-ink/5 bg-surface-muted/50 px-3 py-2 font-mono text-2xs leading-relaxed text-ink-secondary whitespace-pre-wrap",
          className,
        )}
      >
        {body}
      </pre>
    );
  }

  return (
    <div
      className={cn(
        "mt-1 max-h-72 overflow-auto rounded-lg border border-ink/8 bg-[#f7f4ef]",
        className,
      )}
    >
      {stats && (stats.add > 0 || stats.del > 0) ? (
        <div className="sticky top-0 z-[1] flex items-center gap-2 border-b border-ink/6 bg-[#f0ebe3]/95 px-2.5 py-1 text-2xs tabular-nums backdrop-blur-sm">
          <span className="font-medium text-ink-muted">Diff</span>
          <span className="text-emerald-700">+{stats.add}</span>
          <span className="text-ink-muted/50">/</span>
          <span className="text-rose-600">-{stats.del}</span>
        </div>
      ) : null}
      <table className="w-full border-collapse font-mono text-2xs leading-[1.5]">
        <tbody>
          {lines.map((line, i) => {
            const style = KIND_ROW[line.kind];
            let display = line.text;
            if (line.kind === "add" && display.startsWith("+")) {
              display = display.slice(1);
            } else if (line.kind === "del" && display.startsWith("-")) {
              display = display.slice(1);
            }
            return (
              <tr key={`${i}-${line.kind}`} className={style.row}>
                <td
                  className={cn(
                    "w-4 select-none whitespace-nowrap px-1 text-center align-top font-semibold",
                    style.gutter,
                  )}
                >
                  {style.prefix ?? " "}
                </td>
                <td
                  className={cn(
                    "min-w-0 whitespace-pre-wrap break-all pr-2.5 align-top",
                    style.text,
                  )}
                >
                  {display || "\u00a0"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export { isDiffLike };
