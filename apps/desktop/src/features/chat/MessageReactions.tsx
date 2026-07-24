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
 * Reaction pills under a message body (Buzz/Slack grammar — always start-aligned).
 */
export function MessageReactions({
  groups,
  onToggle,
}: {
  groups: ReactionGroup[];
  onToggle: (emoji: string) => void;
}) {
  if (groups.length === 0) return null;

  return (
    <div
      className="mt-1 flex flex-wrap gap-1.5"
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
            "inline-flex h-7 items-center gap-1 rounded-full border px-2 text-xs transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
            group.reactedByMe
              ? "border-accent/40 bg-accent-soft text-ink"
              : "border-ink/10 bg-surface-muted/80 text-ink-secondary hover:bg-surface-hover",
          )}
        >
          <span className="leading-none" aria-hidden>
            {group.emoji}
          </span>
          <span className="min-w-[0.75rem] text-2xs font-medium tabular-nums">
            {group.count}
          </span>
        </button>
      </TooltipTrigger>
      <TooltipContent side="top">{label}</TooltipContent>
    </Tooltip>
  );
}
