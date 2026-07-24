import { useState, type ReactNode } from "react";
import data from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { SmilePlus } from "lucide-react";
import { cn } from "@/shared/lib/utils";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { QUICK_REACTION_EMOJIS } from "./lib/reactions";

type EmojiMartSelection = {
  native?: string;
  shortcodes?: string;
};

/**
 * Quick-react strip + full emoji-mart picker in a popover.
 * Used from message action bar (react) and optionally Composer (insert).
 */
export function EmojiPicker({
  onSelect,
  open,
  onOpenChange,
  trigger,
  side = "top",
  align = "center",
  showQuickStrip = true,
  ariaLabel = "Choose emoji",
}: {
  onSelect: (emoji: string) => void;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Custom trigger; defaults to smile+ icon button. */
  trigger?: ReactNode;
  side?: "top" | "bottom" | "left" | "right";
  align?: "start" | "center" | "end";
  showQuickStrip?: boolean;
  ariaLabel?: string;
}) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const isControlled = open !== undefined;
  const resolvedOpen = isControlled ? open : uncontrolledOpen;
  const setOpen = (next: boolean) => {
    if (!isControlled) setUncontrolledOpen(next);
    onOpenChange?.(next);
  };

  return (
    <Popover open={resolvedOpen} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {trigger ?? (
          <button
            type="button"
            aria-label={ariaLabel}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <SmilePlus className="h-3.5 w-3.5" />
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        className="w-auto max-w-[min(100vw-2rem,360px)] border-ink/10 p-2 shadow-lg"
        onOpenAutoFocus={(e) => e.preventDefault()}
      >
        {showQuickStrip ? (
          <div className="mb-2 flex items-center gap-0.5 border-b border-ink/5 pb-2">
            {QUICK_REACTION_EMOJIS.map((emoji) => (
              <button
                key={emoji}
                type="button"
                aria-label={`React with ${emoji}`}
                onClick={() => {
                  onSelect(emoji);
                  setOpen(false);
                }}
                className={cn(
                  "inline-flex h-8 w-8 items-center justify-center rounded-lg text-[16px]",
                  "hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
                )}
              >
                {emoji}
              </button>
            ))}
          </div>
        ) : null}
        {resolvedOpen ? (
          <div className="emoji-mart-host overflow-hidden rounded-lg [&_em-emoji-picker]:!border-0 [&_em-emoji-picker]:!shadow-none">
            <Picker
              data={data}
              theme="light"
              previewPosition="none"
              skinTonePosition="none"
              navPosition="bottom"
              perLine={8}
              maxFrequentRows={1}
              onEmojiSelect={(emoji: EmojiMartSelection) => {
                const native = emoji.native;
                if (native) {
                  onSelect(native);
                  setOpen(false);
                }
              }}
            />
          </div>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
