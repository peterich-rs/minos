import { cn } from "@/shared/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import {
  reactionActorsLabel,
  type ReactionGroup,
} from "./lib/reactions";

/**
 * Reaction pills under a chat bubble. Click toggles the current user's
 * membership on that emoji (durable local daemon when connected; mock offline).
 */
export function MessageReactions({
  groups,
  onToggle,
  align = "start",
}: {
  groups: ReactionGroup[];
  onToggle: (emoji: string) => void;
  align?: "start" | "end";
}) {
  if (groups.length === 0) return null;

  return (
    <div
      className={cn(
        "flex flex-wrap gap-1 px-0.5",
        align === "end" ? "justify-end" : "justify-start",
      )}
      role="group"
      aria-label="Reactions"
    >
      {groups.map((group) => (
        <ReactionPill
          key={group.emoji}
          group={group}
          onToggle={() => onToggle(group.emoji)}
        />
      ))}
    </div>
  );
}

function ReactionPill({
  group,
  onToggle,
}: {
  group: ReactionGroup;
  onToggle: () => void;
}) {
  const label = reactionActorsLabel(group);
  return (
    <Tooltip delayDuration={200}>
      <TooltipTrigger asChild>
        <button
          type="button"
          onClick={onToggle}
          aria-pressed={group.reactedByMe}
          aria-label={`${group.emoji} ${group.count}. ${label}. Toggle reaction`}
          title={label}
          className={cn(
            "inline-flex h-6 items-center gap-1 rounded-full border px-1.5 text-[12px] transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
            group.reactedByMe
              ? "border-accent/40 bg-accent-soft text-ink"
              : "border-ink/10 bg-surface-muted/80 text-ink-secondary hover:bg-surface-hover",
          )}
        >
          <span className="leading-none" aria-hidden>
            {group.emoji}
          </span>
          <span className="min-w-[0.75rem] text-[11px] font-medium tabular-nums">
            {group.count}
          </span>
        </button>
      </TooltipTrigger>
      <TooltipContent side="top">{label}</TooltipContent>
    </Tooltip>
  );
}
