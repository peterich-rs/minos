/**
 * Shared modal / dialog backdrop tokens.
 * Prefer stronger blur + lighter black tint so the scene stays readable
 * (true frosted glass) instead of a heavy dim plate.
 * Avoid `bg-ink/…` scrims — ink is light in dark themes and turns muddy grey.
 */
export const MODAL_BACKDROP_BLUR_CLASS = "backdrop-blur-[10px]";

/** Light: whisper veil · Dark: soft dim — keep background structure visible. */
export const MODAL_BACKDROP_TINT_CLASS = "bg-black/[0.04] dark:bg-black/25";

export const MODAL_BACKDROP_CLASS = [
  MODAL_BACKDROP_BLUR_CLASS,
  MODAL_BACKDROP_TINT_CLASS,
].join(" ");
