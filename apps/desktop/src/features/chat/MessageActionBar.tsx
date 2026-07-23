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
 * Buttons remain keyboard-focusable (not hover-only traps).
 */
export function MessageActionBar({
  isUser,
  onReply,
  onReact,
  className,
}: {
  isUser: boolean;
  onReply: () => void;
  onReact: (emoji: string) => void;
  className?: string;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);

  return (
    <div
      className={cn(
        "flex items-center gap-0.5 rounded-lg border border-ink/10 bg-surface px-0.5 py-0.5 shadow-sm",
        // Hidden bars must not steal hover/clicks from neighboring rows.
        // Keyboard still tabs in; group-focus-within re-enables hit-testing.
        "pointer-events-none opacity-0 transition-opacity duration-150",
        "group-hover:pointer-events-auto group-hover:opacity-100",
        "group-focus-within:pointer-events-auto group-focus-within:opacity-100",
        // Keep interactive while the emoji popover is open (focus may leave the row).
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
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
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
        align={isUser ? "end" : "start"}
        ariaLabel="Add reaction"
        trigger={
          <button
            type="button"
            aria-label="Add reaction"
            aria-haspopup="dialog"
            aria-expanded={pickerOpen}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <SmilePlus className="h-3.5 w-3.5" />
          </button>
        }
      />
    </div>
  );
}
