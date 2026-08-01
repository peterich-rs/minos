/**
 * Derive Minos CSS design tokens (RGB triplets) from Shiki syntax colors.
 * Preserves Tailwind alpha utilities: `bg-ink/5` → rgb(var(--color-ink) / 0.05).
 */

export type ThemeGitColors = {
  added: string | null;
  deleted: string | null;
  modified: string | null;
};

export type ThemeResult = {
  isDark: boolean;
  vars: Record<string, string>;
};

type RGB = { r: number; g: number; b: number };

function hexToRgb(hex: string): RGB {
  const long = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (long) {
    return {
      r: parseInt(long[1]!, 16),
      g: parseInt(long[2]!, 16),
      b: parseInt(long[3]!, 16),
    };
  }
  const short = /^#?([a-f\d])([a-f\d])([a-f\d])$/i.exec(hex);
  if (short) {
    return {
      r: parseInt(short[1]! + short[1]!, 16),
      g: parseInt(short[2]! + short[2]!, 16),
      b: parseInt(short[3]! + short[3]!, 16),
    };
  }
  return { r: 128, g: 128, b: 128 };
}

function rgbToHex({ r, g, b }: RGB): string {
  const clamp = (n: number) => Math.max(0, Math.min(255, Math.round(n)));
  return `#${[r, g, b].map((c) => clamp(c).toString(16).padStart(2, "0")).join("")}`;
}

function toTriplet(hex: string): string {
  const { r, g, b } = hexToRgb(hex);
  return `${r} ${g} ${b}`;
}

export function luminance(hex: string): number {
  const { r, g, b } = hexToRgb(hex);
  const [rs, gs, bs] = [r, g, b].map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rs! + 0.7152 * gs! + 0.0722 * bs!;
}

function mix(hex1: string, hex2: string, factor: number): string {
  const c1 = hexToRgb(hex1);
  const c2 = hexToRgb(hex2);
  return rgbToHex({
    r: c1.r + (c2.r - c1.r) * factor,
    g: c1.g + (c2.g - c1.g) * factor,
    b: c1.b + (c2.b - c1.b) * factor,
  });
}

function adjust(hex: string, amount: number): string {
  const target = amount > 0 ? "#ffffff" : "#000000";
  return mix(hex, target, Math.abs(amount));
}

/**
 * Build Minos surface/ink tokens from syntax theme key colors.
 * `minos` theme applies a warm canvas tint after this base derivation.
 */
export function createThemeVars(
  syntaxBg: string,
  syntaxFg: string,
  syntaxComment: string,
  options?: {
    warmCanvas?: boolean;
    accentHex?: string;
    gitColors?: ThemeGitColors;
  },
): ThemeResult {
  const isDark = luminance(syntaxBg) < 0.5;
  const dir = isDark ? 1 : -1;
  const surface = syntaxBg;
  const canvas = options?.warmCanvas
    ? isDark
      ? mix(surface, "#3f3a36", 0.12)
      : mix(surface, "#ebe4d8", 0.55)
    : isDark
      ? adjust(surface, -0.04)
      : mix(surface, "#ebe4d8", 0.35);
  const surfaceMuted = adjust(surface, dir * 0.04);
  const surfaceHover = adjust(surface, dir * 0.06);
  // Raised cards need enough lift on dark themes (pure white cards were the
  // multi-theme bug — components must use bg-surface-raised, not bg-white).
  const surfaceRaised = isDark ? adjust(surface, 0.1) : "#ffffff";
  const ink = syntaxFg;
  const inkSecondary = mix(syntaxFg, syntaxComment, 0.35);
  const inkMuted = syntaxComment;
  const inkFaint = mix(syntaxComment, surface, 0.45);

  const accent =
    options?.accentHex ?? (isDark ? "#f472b6" : "#ec4899");
  const accentStrong = mix(accent, isDark ? "#ffffff" : "#000000", 0.15);
  const accentSoft = mix(surface, accent, isDark ? 0.22 : 0.12);

  // Mauve primary (Buzz / Catppuccin) — separate from pink accent.
  const primary = isDark ? "#cba6f7" : "#9333ea";
  const primaryStrong = isDark ? "#e0b0ff" : "#7e22ce";
  const primarySoft = mix(surface, primary, isDark ? 0.28 : 0.14);

  const git = options?.gitColors;
  const statusRunning = git?.modified ?? (isDark ? "#d29922" : "#f59e0b");
  const statusFailed = git?.deleted ?? (isDark ? "#f85149" : "#dc2626");
  const statusDone = git?.added ?? (isDark ? "#3fb950" : "#10b981");

  return {
    isDark,
    vars: {
      "--color-canvas": toTriplet(canvas),
      "--color-canvas-soft": toTriplet(mix(canvas, surface, 0.5)),
      "--color-surface": toTriplet(surface),
      "--color-surface-raised": toTriplet(surfaceRaised),
      "--color-surface-muted": toTriplet(surfaceMuted),
      "--color-surface-hover": toTriplet(surfaceHover),
      "--color-ink": toTriplet(ink),
      "--color-ink-secondary": toTriplet(inkSecondary),
      "--color-ink-muted": toTriplet(inkMuted),
      "--color-ink-faint": toTriplet(inkFaint),
      "--color-accent": toTriplet(accent),
      "--color-accent-strong": toTriplet(accentStrong),
      "--color-accent-soft": toTriplet(accentSoft),
      "--color-primary": toTriplet(primary),
      "--color-primary-strong": toTriplet(primaryStrong),
      "--color-primary-soft": toTriplet(primarySoft),
      "--color-bubble-out": toTriplet(ink),
      "--color-bubble-in": toTriplet(surfaceRaised),
      "--color-status-idle": toTriplet(inkMuted),
      "--color-status-running": toTriplet(statusRunning),
      "--color-status-approval": toTriplet(isDark ? "#fb7185" : "#f43f5e"),
      "--color-status-suspended": toTriplet(isDark ? "#38bdf8" : "#0ea5e9"),
      "--color-status-failed": toTriplet(statusFailed),
      "--color-status-done": toTriplet(statusDone),
      // Shell gradient (Buzz dual-layer; CSS toggles via html.dark)
      "--buzz-gradient-light-top": isDark ? "#4a4616" : "#e6e6b6",
      "--buzz-gradient-light-bottom": isDark ? "#0a1423" : "#c4d0da",
      // Markdown tones
      "--md-link": isDark ? "#fcd34d" : "#92400e",
      "--md-code-bg": isDark
        ? "rgb(255 255 255 / 0.08)"
        : "rgb(28 25 23 / 0.07)",
      "--md-code-fg": isDark ? "#e7e5e4" : "#44403c",
      "--md-pre-bg": isDark
        ? "rgb(255 255 255 / 0.05)"
        : "rgb(28 25 23 / 0.05)",
      "--md-pre-fg": isDark ? "#d6d3d1" : "#3f3a36",
      "--shadow-content-edge": isDark
        ? "-1px -1px 0 0 rgb(255 255 255 / 0.06), 0 1px 2px rgb(0 0 0 / 0.35)"
        : "-1px -1px 0 0 rgb(28 25 23 / 0.08), 0 1px 2px rgb(28 25 23 / 0.04)",
    },
  };
}
