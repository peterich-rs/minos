import { useState } from "react";
import { Reply, SmilePlus } from "lucide-react";
import { cn } from "@/shared/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { EmojiPicker } from "./EmojiPicker";

/**
 * Hover / focus action bar on a message row: Reply + React.
 * Buzz-style floating pill; buttons remain keyboard-focusable.
 */
export function MessageActionBar({
  onReply,
  onReact,
  className,
}: {
  onReply: () => void;
  onReact: (emoji: string) => void;
  className?: string;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);

  return (
    <div
      className={cn(
        "flex items-center gap-0.5 rounded-full border border-ink/10 bg-surface/95 px-0.5 py-0.5 shadow-sm backdrop-blur-sm",
        // Hidden bars must not steal hover/clicks from neighboring rows.
        "pointer-events-none opacity-0 transition-opacity duration-150",
        "group-hover/message:pointer-events-auto group-hover/message:opacity-100",
        "group-focus-within/message:pointer-events-auto group-focus-within/message:opacity-100",
        pickerOpen && "pointer-events-auto opacity-100",
        className,
      )}
      role="toolbar"
      aria-label="Message actions"
    >
      <Tooltip delayDuration={300}>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={onReply}
            aria-label="Reply"
            className="inline-flex h-7 w-7 items-center justify-center rounded-full text-ink-muted hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <Reply className="h-3.5 w-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Reply</TooltipContent>
      </Tooltip>

      <EmojiPicker
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        onSelect={onReact}
        side="top"
        align="end"
        ariaLabel="Add reaction"
        trigger={
          <button
            type="button"
            aria-label="Add reaction"
            aria-haspopup="dialog"
            aria-expanded={pickerOpen}
            className="inline-flex h-7 w-7 items-center justify-center rounded-full text-ink-muted hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <SmilePlus className="h-3.5 w-3.5" />
          </button>
        }
      />
    </div>
  );
}
