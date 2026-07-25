import type { ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "@/shared/lib/utils";
import { Button } from "@/shared/ui/button";

type SidebarActionCardProps = {
  title: string;
  description?: string;
  icon?: ReactNode;
  /** Primary action label (e.g. Retry). */
  actionLabel: string;
  onAction: () => void;
  actionDisabled?: boolean;
  onDismiss?: () => void;
  dismissLabel?: string;
  /** Optional secondary action (e.g. Open Host). */
  secondaryLabel?: string;
  onSecondary?: () => void;
  tone?: "neutral" | "danger" | "success";
  testId?: string;
  className?: string;
  role?: "alert" | "status";
};

const toneRing: Record<NonNullable<SidebarActionCardProps["tone"]>, string> = {
  neutral: "border-ink/10 bg-surface-muted/80",
  danger: "border-status-failed/30 bg-status-failed/10",
  success: "border-status-done/30 bg-status-done/10",
};

/**
 * Compact sidebar status/action card (connection offline, nudges, etc.).
 */
export function SidebarActionCard({
  title,
  description,
  icon,
  actionLabel,
  onAction,
  actionDisabled,
  onDismiss,
  dismissLabel = "Dismiss",
  secondaryLabel,
  onSecondary,
  tone = "neutral",
  testId,
  className,
  role = "status",
}: SidebarActionCardProps) {
  return (
    <div
      role={role}
      data-testid={testId}
      className={cn(
        "relative rounded-xl border px-3 py-2.5 shadow-sm",
        toneRing[tone],
        className,
      )}
    >
      {onDismiss ? (
        <button
          type="button"
          aria-label={dismissLabel}
          onClick={onDismiss}
          className="absolute right-1.5 top-1.5 rounded-md p-1 text-ink-muted transition-colors hover:bg-surface-hover hover:text-ink"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      ) : null}

      <div className={cn("flex gap-2", onDismiss && "pr-5")}>
        {icon ? (
          <div className="mt-0.5 shrink-0 text-ink-secondary">{icon}</div>
        ) : null}
        <div className="min-w-0 flex-1">
          <div className="text-xs font-semibold text-ink">{title}</div>
          {description ? (
            <p className="mt-0.5 text-2xs leading-snug text-ink-muted">
              {description}
            </p>
          ) : null}
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <Button
              type="button"
              size="sm"
              variant="default"
              disabled={actionDisabled}
              onClick={onAction}
              className="h-7 rounded-lg px-2.5 text-2xs"
            >
              {actionLabel}
            </Button>
            {secondaryLabel && onSecondary ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={onSecondary}
                className="h-7 rounded-lg px-2 text-2xs"
              >
                {secondaryLabel}
              </Button>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
