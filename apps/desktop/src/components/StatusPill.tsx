import { cn } from "@/lib/utils";
import { statusMeta, type SessionStatus } from "@/lib/mock-data";

export function StatusPill({
  status,
  className,
}: {
  status: SessionStatus;
  className?: string;
}) {
  const meta = statusMeta[status];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium",
        meta.pill,
        className,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", meta.dot)} />
      {meta.label}
    </span>
  );
}
