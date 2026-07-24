import { Loader2 } from "lucide-react";
import { cn } from "@/shared/lib/utils";
import { statusMeta, type SessionStatus } from "@/shared/lib/mock-data";

export function StatusPill({
  status,
  className,
}: {
  status: SessionStatus | string;
  className?: string;
}) {
  // Runtime status can be unexpected wire strings — never throw on render.
  const meta = statusMeta[status as SessionStatus] ?? statusMeta.idle;
  const executing = status === "running" || status === "needs_approval";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium",
        meta.pill,
        className,
      )}
    >
      {executing ? (
        <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
      ) : (
        <span className={cn("h-1.5 w-1.5 rounded-full", meta.dot)} />
      )}
      {meta.label}
    </span>
  );
}
