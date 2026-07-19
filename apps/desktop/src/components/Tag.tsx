import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import type {
  ConversationPriority,
  ConversationProgress,
} from "@/lib/mock-data";

const priorityStyles: Record<
  ConversationPriority,
  { label: string; className: string; dot: string }
> = {
  high: {
    label: "High",
    className: "bg-rose-50 text-rose-700 ring-rose-200/80",
    dot: "bg-rose-500",
  },
  medium: {
    label: "Medium",
    className: "bg-amber-50 text-amber-800 ring-amber-200/80",
    dot: "bg-amber-500",
  },
  low: {
    label: "Low",
    className: "bg-sky-50 text-sky-700 ring-sky-200/80",
    dot: "bg-sky-500",
  },
};

const progressStyles: Record<
  ConversationProgress,
  { label: string; className: string }
> = {
  todo: {
    label: "To do",
    className: "bg-stone-100 text-stone-600 ring-stone-200/80",
  },
  in_progress: {
    label: "In progress",
    className: "bg-violet-50 text-violet-700 ring-violet-200/80",
  },
  in_review: {
    label: "In review",
    className: "bg-indigo-50 text-indigo-700 ring-indigo-200/80",
  },
  done: {
    label: "Done",
    className: "bg-emerald-50 text-emerald-700 ring-emerald-200/80",
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
    size === "sm" ? "px-1.5 py-0.5 text-[10px]" : "px-2 py-0.5 text-[11px]",
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
    size === "sm" ? "px-1.5 py-0.5 text-[10px]" : "px-2 py-0.5 text-[11px]",
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
    size === "sm" ? "px-1.5 py-0.5 text-[10px]" : "px-2 py-0.5 text-[11px]",
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
        "inline-flex max-w-full items-center gap-1 truncate rounded-md bg-surface-muted px-2 py-0.5 font-mono text-[11px] text-ink-secondary ring-1 ring-inset ring-ink/5",
        className,
      )}
    >
      {children}
    </span>
  );
}
