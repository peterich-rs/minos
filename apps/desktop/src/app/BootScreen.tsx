import { Sparkles } from "lucide-react";
import { cn } from "@/shared/lib/utils";

type Props = {
  phase: string;
  /** 0–100 */
  progress: number;
};

export function BootScreen({ phase, progress }: Props) {
  const pct = Math.max(0, Math.min(100, progress));

  return (
    <div className="relative flex h-full min-h-full w-full flex-col items-center justify-center overflow-hidden px-8">
      <div className="minos-theme-gradient" aria-hidden />
      <div className="minos-theme-grain" aria-hidden />
      <div className="relative z-10 flex w-full max-w-sm flex-col items-center rounded-2xl border border-ink/8 bg-surface/90 px-8 py-10 shadow-shell backdrop-blur-md">
        <div className="mb-6 flex h-14 w-14 items-center justify-center rounded-2xl bg-ink text-surface shadow-md">
          <Sparkles className="h-7 w-7" strokeWidth={2} />
        </div>
        <div className="text-base font-semibold tracking-tight text-ink">
          Minos
        </div>
        <p className="mt-1.5 text-sm text-ink-muted">{phase}</p>

        <div className="mt-8 w-full">
          <div className="h-1.5 overflow-hidden rounded-full bg-surface-muted ring-1 ring-ink/5">
            <div
              className={cn(
                "h-full rounded-full bg-primary transition-[width] duration-300 ease-out",
              )}
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="mt-2 text-center text-2xs tabular-nums text-ink-muted">
            {Math.round(pct)}%
          </div>
        </div>
      </div>
    </div>
  );
}
