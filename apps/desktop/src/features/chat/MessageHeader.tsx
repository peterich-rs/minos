import type { ReactNode } from "react";
import { cn } from "@/shared/lib/utils";

/** Buzz-style header row: author + time + metadata baseline. */
export function MessageHeaderRow({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0 leading-4",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function MessageAuthorText({
  as: Component = "span",
  children,
  className,
  hoverUnderline = false,
}: {
  as?: "div" | "h3" | "span" | "button";
  children: ReactNode;
  className?: string;
  hoverUnderline?: boolean;
}) {
  return (
    <Component
      className={cn(
        "truncate text-sm font-semibold leading-4 tracking-tight text-ink",
        hoverUnderline && "hover:underline",
        className,
      )}
      data-testid="message-author"
    >
      {children}
    </Component>
  );
}
