/**
 * Shiki theme loader (subset) — Buzz-inspired multi-theme system.
 * Only imports theme JSON; highlighter engine loads languages lazily in CodeBlock.
 */

import type { ThemeRegistrationRaw } from "shiki";

export const MINOS_THEME_NAME = "minos";
export const MINOS_BASE_THEME = "github-light" as const;

/** Themes offered in Host → Appearance (keep small for bundle). */
export const SYNTAX_THEMES = [
  "minos",
  "github-light",
  "github-dark",
  "catppuccin-latte",
  "catppuccin-mocha",
  "one-dark-pro",
  "nord",
  "solarized-light",
  "tokyo-night",
] as const;

export type SyntaxThemeName = (typeof SYNTAX_THEMES)[number];

export const LIGHT_THEMES: ReadonlySet<string> = new Set([
  "minos",
  "github-light",
  "catppuccin-latte",
  "solarized-light",
]);

const themeImports: Record<
  Exclude<SyntaxThemeName, "minos"> | "github-light",
  () => Promise<{ default: ThemeRegistrationRaw }>
> = {
  "github-light": () => import("shiki/themes/github-light.mjs"),
  "github-dark": () => import("shiki/themes/github-dark.mjs"),
  "catppuccin-latte": () => import("shiki/themes/catppuccin-latte.mjs"),
  "catppuccin-mocha": () => import("shiki/themes/catppuccin-mocha.mjs"),
  "one-dark-pro": () => import("shiki/themes/one-dark-pro.mjs"),
  nord: () => import("shiki/themes/nord.mjs"),
  "solarized-light": () => import("shiki/themes/solarized-light.mjs"),
  "tokyo-night": () => import("shiki/themes/tokyo-night.mjs"),
};

export function resolveShikiThemeName(name: string): string {
  if (name === MINOS_THEME_NAME) return MINOS_BASE_THEME;
  return name;
}

export function isValidThemeName(name: string): name is SyntaxThemeName {
  return (SYNTAX_THEMES as readonly string[]).includes(name);
}

export function isLightTheme(name: string): boolean {
  return LIGHT_THEMES.has(name);
}

export type ThemeInfo = {
  name: string;
  bg: string;
  fg: string;
  comment: string;
  gitAdded: string | null;
  gitDeleted: string | null;
  gitModified: string | null;
};

type ThemeSetting = {
  scope?: string | string[];
  settings?: { foreground?: string };
};

function extractCommentColor(
  settings: ReadonlyArray<ThemeSetting> | undefined,
  fg: string,
): string {
  if (!settings) return fg;
  for (const entry of settings) {
    const scope = entry.scope;
    const scopes = Array.isArray(scope) ? scope : scope ? [scope] : [];
    if (
      scopes.some(
        (s) =>
          s === "comment" ||
          s.startsWith("comment.") ||
          s.includes("comment"),
      )
    ) {
      const color = entry.settings?.foreground;
      if (color) return color;
    }
  }
  return fg;
}

function extractGitColors(colors: Record<string, string> | undefined): {
  gitAdded: string | null;
  gitDeleted: string | null;
  gitModified: string | null;
} {
  if (!colors) {
    return { gitAdded: null, gitDeleted: null, gitModified: null };
  }
  return {
    gitAdded:
      colors["gitDecoration.addedResourceForeground"] ??
      colors["editorGutter.addedBackground"] ??
      null,
    gitDeleted:
      colors["gitDecoration.deletedResourceForeground"] ??
      colors["editorGutter.deletedBackground"] ??
      null,
    gitModified:
      colors["gitDecoration.modifiedResourceForeground"] ??
      colors["editorGutter.modifiedBackground"] ??
      null,
  };
}

export function extractThemeInfo(
  themeName: string,
  theme: ThemeRegistrationRaw,
): ThemeInfo {
  const colors = theme.colors as Record<string, string> | undefined;
  const bg = colors?.["editor.background"] ?? "#1e1e1e";
  const fg = colors?.["editor.foreground"] ?? "#d4d4d4";
  return {
    name: themeName,
    bg,
    fg,
    comment: extractCommentColor(
      theme.settings as ReadonlyArray<ThemeSetting> | undefined,
      fg,
    ),
    ...extractGitColors(colors),
  };
}

export async function loadThemeData(
  name: SyntaxThemeName,
): Promise<ThemeRegistrationRaw> {
  const resolved = resolveShikiThemeName(name) as Exclude<
    SyntaxThemeName,
    "minos"
  >;
  const loader = themeImports[resolved] ?? themeImports["github-light"];
  const { default: theme } = await loader();
  return theme;
}

/** Human labels for the theme picker. */
export const THEME_LABELS: Record<SyntaxThemeName, string> = {
  minos: "Minos (warm)",
  "github-light": "GitHub Light",
  "github-dark": "GitHub Dark",
  "catppuccin-latte": "Catppuccin Latte",
  "catppuccin-mocha": "Catppuccin Mocha",
  "one-dark-pro": "One Dark Pro",
  nord: "Nord",
  "solarized-light": "Solarized Light",
  "tokyo-night": "Tokyo Night",
};
