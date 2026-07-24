import { useMemo } from "react";
import { cn } from "@/shared/lib/utils";
import {
  isGrokArrowNumbered,
  parseGrokArrowNumberedLines,
  type NumberedLine,
} from "@/shared/lib/read-lines";

/**
 * File-read body with a line-number gutter.
 * Parses Grok's `N→content` markers into real file line numbers so the arrow
 * never appears inline with source.
 */
export function ReadView({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  const body = typeof text === "string" ? text : "";
  const lines = useMemo((): NumberedLine[] => {
    if (!body.trim()) return [];
    const parsed = parseGrokArrowNumberedLines(body);
    if (parsed) return parsed;
    // Plain body: synthetic 1-based gutter.
    const raw = body.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
    if (raw.length > 0 && raw[raw.length - 1] === "") raw.pop();
    return raw.map((text, i) => ({ no: i + 1, text }));
  }, [body]);

  if (!body.trim()) return null;

  const gutterW = String(lines[lines.length - 1]?.no ?? 1).length;

  return (
    <div
      className={cn(
        "mt-1 max-h-72 overflow-auto rounded-lg border border-ink/5 bg-surface-muted/50 font-mono text-2xs leading-[1.5]",
        className,
      )}
    >
      <table className="w-full border-collapse">
        <tbody>
          {lines.map((line) => (
            <tr key={`${line.no}-${line.text.slice(0, 24)}`}>
              <td
                className="select-none whitespace-nowrap border-r border-ink/6 px-2 py-0 text-right tabular-nums text-ink-muted/55 align-top"
                style={{ minWidth: `${gutterW + 2}ch` }}
              >
                {line.no}
              </td>
              <td className="whitespace-pre-wrap break-all px-2 py-0 text-ink-secondary">
                {line.text || "\u00a0"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function shouldUseReadView(opts: {
  toolName: string;
  detail: string;
  isDiff: boolean;
}): boolean {
  if (opts.isDiff || !opts.detail.trim()) return false;
  if (isGrokArrowNumbered(opts.detail)) return true;
  const n = opts.toolName.toLowerCase();
  return (
    n.includes("read") ||
    n.startsWith("read:") ||
    n.includes("read_file") ||
    n.includes("readfile")
  );
}
