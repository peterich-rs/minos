import { cn } from "@/shared/lib/utils";

/**
 * Compact message timestamp (Buzz grammar).
 * Use `title` for full datetime when available.
 */
export function MessageTimestamp({
  time,
  title,
  className,
}: {
  time: string;
  title?: string;
  className?: string;
}) {
  return (
    <time
      className={cn(
        "text-xs font-normal leading-4 tabular-nums text-ink-muted/70",
        className,
      )}
      title={title}
      dateTime={title}
    >
      {time}
    </time>
  );
}
