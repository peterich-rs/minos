import type { ReactNode } from "react";
import { cn } from "@/shared/lib/utils";
import type {
  ConversationPriority,
  ConversationProgress,
} from "../domain/collaboration.ts";

const priorityStyles: Record<
  ConversationPriority,
  { label: string; className: string; dot: string }
> = {
  high: {
    label: "High",
    className: "bg-status-failed/15 text-status-failed ring-status-failed/30",
    dot: "bg-rose-500",
  },
  medium: {
    label: "Medium",
    className: "bg-status-running/15 text-status-running ring-status-running/30",
    dot: "bg-status-running",
  },
  low: {
    label: "Low",
    className:
      "bg-status-suspended/15 text-status-suspended ring-status-suspended/30",
    dot: "bg-status-suspended",
  },
};

const progressStyles: Record<
  ConversationProgress,
  { label: string; className: string }
> = {
  todo: {
    label: "To do",
    className: "bg-ink/10 text-ink-secondary ring-ink/10",
  },
  in_progress: {
    label: "In progress",
    className: "bg-violet-500/15 text-violet-800 ring-violet-500/25 dark:text-violet-200",
  },
  in_review: {
    label: "In review",
    className: "bg-indigo-500/15 text-indigo-800 ring-indigo-500/25 dark:text-indigo-200",
  },
  done: {
    label: "Done",
    className: "bg-status-done/15 text-status-done ring-status-done/30",
  },
};

export function PriorityTag({
  priority,
  size = "md",
  onClick,
  title,
}: {
  priority: ConversationPriority;
  size?: "sm" | "md";
  onClick?: () => void;
  title?: string;
}) {
  const meta = priorityStyles[priority];
  const className = cn(
    "inline-flex items-center gap-1 rounded-md font-medium ring-1 ring-inset",
    size === "sm" ? "px-1.5 py-0.5 text-3xs" : "px-2 py-0.5 text-2xs",
    meta.className,
    onClick && "cursor-pointer transition-opacity hover:opacity-80",
  );
  const body = (
    <>
      <span className={cn("h-1.5 w-1.5 rounded-full", meta.dot)} />
      {meta.label}
    </>
  );
  if (onClick) {
    return (
      <button type="button" onClick={onClick} title={title} className={className}>
        {body}
      </button>
    );
  }
  return (
    <span className={className} title={title}>
      {body}
    </span>
  );
}

/** Unset priority placeholder — click to set High. */
export function PriorityPlaceholder({
  size = "md",
  onClick,
}: {
  size?: "sm" | "md";
  onClick?: () => void;
}) {
  const className = cn(
    "inline-flex items-center gap-1 rounded-md font-medium text-ink-muted ring-1 ring-inset ring-ink/10",
    size === "sm" ? "px-1.5 py-0.5 text-3xs" : "px-2 py-0.5 text-2xs",
    onClick && "cursor-pointer transition-colors hover:bg-surface-muted hover:text-ink",
  );
  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        title="Set priority"
        className={className}
      >
        Priority
      </button>
    );
  }
  return <span className={className}>Priority</span>;
}

export function ProgressTag({
  progress,
  size = "md",
  onClick,
  title,
}: {
  progress: ConversationProgress;
  size?: "sm" | "md";
  onClick?: () => void;
  title?: string;
}) {
  const meta = progressStyles[progress];
  const className = cn(
    "inline-flex items-center rounded-md font-medium ring-1 ring-inset",
    size === "sm" ? "px-1.5 py-0.5 text-3xs" : "px-2 py-0.5 text-2xs",
    meta.className,
    onClick && "cursor-pointer transition-opacity hover:opacity-80",
  );
  if (onClick) {
    return (
      <button type="button" onClick={onClick} title={title} className={className}>
        {meta.label}
      </button>
    );
  }
  return (
    <span className={className} title={title}>
      {meta.label}
    </span>
  );
}

export function MetaChip({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex max-w-full items-center gap-1 truncate rounded-md bg-surface-muted px-2 py-0.5 font-mono text-2xs text-ink-secondary ring-1 ring-inset ring-ink/5",
        className,
      )}
    >
      {children}
    </span>
  );
}
