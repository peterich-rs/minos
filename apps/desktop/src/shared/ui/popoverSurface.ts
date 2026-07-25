/**
 * Shared popover / dropdown surface + motion tokens.
 */

export const POPOVER_SURFACE_CLASS =
  "rounded-xl border border-ink/10 bg-surface text-ink shadow-lg outline-none";

export const POPOVER_RADIX_MOTION_CLASS =
  "duration-150 ease-out data-[state=closed]:duration-100 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 motion-reduce:animate-none";

export const POPOVER_RADIX_SIDE_MOTION_CLASS =
  "data-[side=bottom]:slide-in-from-top-1 data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1 data-[side=top]:slide-in-from-bottom-1";
