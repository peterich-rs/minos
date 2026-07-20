import { Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";

type Props = {
  phase: string;
  /** 0–100 */
  progress: number;
};

export function BootScreen({ phase, progress }: Props) {
  const pct = Math.max(0, Math.min(100, progress));

  return (
    <div className="flex h-full min-h-full w-full flex-col items-center justify-center bg-surface px-8">
      <div className="mb-6 flex h-14 w-14 items-center justify-center rounded-2xl bg-ink text-white shadow-md">
        <Sparkles className="h-7 w-7" strokeWidth={2} />
      </div>
      <div className="text-[16px] font-semibold tracking-tight text-ink">
        Minos
      </div>
      <p className="mt-1.5 text-[13px] text-ink-muted">{phase}</p>

      <div className="mt-8 w-full max-w-xs">
        <div className="h-1.5 overflow-hidden rounded-full bg-surface-muted ring-1 ring-ink/5">
          <div
            className={cn(
              "h-full rounded-full bg-ink transition-[width] duration-300 ease-out",
            )}
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="mt-2 text-center text-[11px] tabular-nums text-ink-muted">
          {Math.round(pct)}%
        </div>
      </div>
    </div>
  );
}
