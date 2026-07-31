import type { ReactNode } from "react";
import { PauseCircle, ShieldAlert, XCircle } from "lucide-react";

import { cn } from "@/shared/lib/utils";

export type AttentionTone = "approval" | "failed" | "suspended" | "neutral";

/**
 * Attention queue card shared by Desktop AttentionView + Web CloudAttentionView.
 */
export function AttentionListCard({
  tone = "neutral",
  icon,
  title,
  badge,
  body,
  meta,
  actions,
  className,
}: {
  tone?: AttentionTone;
  icon?: ReactNode;
  title: ReactNode;
  badge?: ReactNode;
  body?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  const defaultIcon =
    tone === "approval" ? (
      <ShieldAlert className="h-4 w-4" />
    ) : tone === "failed" ? (
      <XCircle className="h-4 w-4" />
    ) : tone === "suspended" ? (
      <PauseCircle className="h-4 w-4" />
    ) : null;

  return (
    <div
      className={cn(
        "rounded-2xl border border-ink/6 bg-surface p-4 shadow-panel transition-shadow hover:shadow-sm",
        className,
      )}
    >
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "flex h-9 w-9 shrink-0 items-center justify-center rounded-xl",
            tone === "approval" && "bg-rose-100 text-rose-700",
            tone === "failed" && "bg-red-100 text-red-700",
            tone === "suspended" &&
              "bg-status-suspended/15 text-status-suspended",
            tone === "neutral" && "bg-primary-soft text-primary",
          )}
        >
          {icon ?? defaultIcon}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-ink">{title}</span>
            {badge}
          </div>
          {body ? (
            <p className="mt-1 text-sm text-ink-secondary">{body}</p>
          ) : null}
          {meta ? (
            <p className="mt-1 text-xs text-ink-muted">{meta}</p>
          ) : null}
          {actions ? (
            <div className="mt-3 flex flex-wrap gap-2">{actions}</div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function AttentionPrimaryButton({
  children,
  onClick,
  disabled,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-white shadow-sm hover:opacity-90 disabled:opacity-45"
    >
      {children}
    </button>
  );
}

export function AttentionSecondaryButton({
  children,
  onClick,
  disabled,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded-lg border border-ink/10 bg-surface px-3 py-1.5 text-xs font-medium text-ink shadow-sm hover:bg-surface-hover disabled:opacity-45"
    >
      {children}
    </button>
  );
}
