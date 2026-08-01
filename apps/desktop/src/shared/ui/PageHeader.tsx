import type { ButtonHTMLAttributes, ReactNode } from "react";

import { cn } from "@/shared/lib/utils";

/**
 * Shared page header ramp (Buzz PageHeader grammar).
 * One `h1` per page; optional description + trailing action.
 */
export function PageHeader({
  title,
  description,
  action,
  className,
  badge,
}: {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  badge?: ReactNode;
  className?: string;
}) {
  const copy = (
    <>
      <div className="flex min-w-0 flex-wrap items-center gap-2.5">
        <h1 className="text-2xl font-semibold tracking-tight text-ink">
          {title}
        </h1>
        {badge}
      </div>
      {description ? (
        <p className="mt-1 max-w-2xl text-sm leading-relaxed text-ink-muted">
          {description}
        </p>
      ) : null}
    </>
  );

  return (
    <header
      className={cn(
        "shrink-0 border-b border-ink/6 bg-surface/95 px-5 py-5 backdrop-blur-sm sm:px-6",
        className,
      )}
    >
      {action ? (
        <div className="flex min-w-0 items-start justify-between gap-4">
          <div className="min-w-0">{copy}</div>
          <div className="shrink-0">{action}</div>
        </div>
      ) : (
        <div className="min-w-0">{copy}</div>
      )}
    </header>
  );
}

/** Primary solid CTA used in page headers (Create / Reconnect). */
export function PageHeaderPrimaryButton({
  children,
  className,
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type={type}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-xl bg-primary px-3.5 py-2 text-xs font-semibold text-white shadow-sm transition-opacity hover:opacity-90 disabled:opacity-40",
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

/** Secondary outline / muted header button. */
export function PageHeaderSecondaryButton({
  children,
  className,
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type={type}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-xl border border-ink/10 bg-surface px-3 py-2 text-xs font-semibold text-ink shadow-sm transition-colors hover:bg-surface-hover disabled:opacity-40",
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}
