import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createThemeVars } from "./adaptive-theme";
import {
  extractThemeInfo,
  isValidThemeName,
  loadThemeData,
  MINOS_THEME_NAME,
  type SyntaxThemeName,
  SYNTAX_THEMES,
} from "./theme-loader";

export const THEME_STORAGE_KEY = "minos-theme";
export const THEME_CACHE_KEY = "minos-theme-cache";
export const ACCENT_STORAGE_KEY = "minos-accent-color";
export const NEUTRAL_ACCENT = "neutral";

export const ACCENT_COLORS = [
  { name: "Neutral", value: NEUTRAL_ACCENT },
  { name: "Pink", value: "#ec4899" },
  { name: "Blue", value: "#3b82f6" },
  { name: "Cyan", value: "#06b6d4" },
  { name: "Green", value: "#22c55e" },
  { name: "Orange", value: "#f97316" },
  { name: "Purple", value: "#a855f7" },
] as const;

const DEFAULT_THEME: SyntaxThemeName = MINOS_THEME_NAME;
const DEFAULT_ACCENT = "#ec4899";

type ThemeContextValue = {
  themeName: string;
  isDark: boolean;
  isLoading: boolean;
  accentColor: string;
  themes: readonly SyntaxThemeName[];
  setTheme: (name: string) => void;
  setAccentColor: (color: string) => void;
};

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function readStoredTheme(): SyntaxThemeName {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored && isValidThemeName(stored)) return stored;
  } catch {
    /* ignore */
  }
  return DEFAULT_THEME;
}

function readStoredAccent(): string {
  try {
    return window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
  } catch {
    return DEFAULT_ACCENT;
  }
}

function applyVars(vars: Record<string, string>, isDark: boolean) {
  const root = document.documentElement;
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value);
  }
  root.classList.toggle("dark", isDark);
  root.style.colorScheme = isDark ? "dark" : "light";
}

/** Sync FOUC path: re-apply cached vars before first paint when possible. */
export function applyCachedThemeVars() {
  try {
    const raw = window.localStorage.getItem(THEME_CACHE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as {
      vars?: Record<string, string>;
      isDark?: boolean;
    };
    if (parsed.vars) {
      applyVars(parsed.vars, Boolean(parsed.isDark));
    }
  } catch {
    /* ignore corrupt cache */
  }
}

// Run once at module load in the browser.
if (typeof window !== "undefined") {
  applyCachedThemeVars();
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [themeName, setThemeName] = useState<SyntaxThemeName>(readStoredTheme);
  const [accentColor, setAccentColorState] = useState(readStoredAccent);
  const [isDark, setIsDark] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const requestId = useRef(0);

  const applyTheme = useCallback(async (name: SyntaxThemeName, accent: string) => {
    const id = ++requestId.current;
    setIsLoading(true);
    try {
      const data = await loadThemeData(name);
      if (id !== requestId.current) return;
      const info = extractThemeInfo(name, data);
      const accentHex = accent === NEUTRAL_ACCENT ? undefined : accent;
      const result = createThemeVars(info.bg, info.fg, info.comment, {
        warmCanvas: name === MINOS_THEME_NAME,
        accentHex,
        gitColors: {
          added: info.gitAdded,
          deleted: info.gitDeleted,
          modified: info.gitModified,
        },
      });
      applyVars(result.vars, result.isDark);
      setIsDark(result.isDark);
      try {
        window.localStorage.setItem(
          THEME_CACHE_KEY,
          JSON.stringify({ vars: result.vars, isDark: result.isDark, name }),
        );
      } catch {
        /* quota */
      }
    } catch (err) {
      console.warn("[theme] failed to load", name, err);
    } finally {
      if (id === requestId.current) setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void applyTheme(themeName, accentColor);
  }, [themeName, accentColor, applyTheme]);

  const setTheme = useCallback((name: string) => {
    if (!isValidThemeName(name)) return;
    setThemeName(name);
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, name);
    } catch {
      /* ignore */
    }
  }, []);

  const setAccentColor = useCallback((color: string) => {
    setAccentColorState(color);
    try {
      window.localStorage.setItem(ACCENT_STORAGE_KEY, color);
    } catch {
      /* ignore */
    }
  }, []);

  const value = useMemo(
    () => ({
      themeName,
      isDark,
      isLoading,
      accentColor,
      themes: SYNTAX_THEMES,
      setTheme,
      setAccentColor,
    }),
    [themeName, isDark, isLoading, accentColor, setTheme, setAccentColor],
  );

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error("useTheme must be used within ThemeProvider");
  }
  return ctx;
}

/** Safe for surfaces that may render outside ThemeProvider (tests). */
export function useThemeOptional(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (ctx) return ctx;
  return {
    themeName: DEFAULT_THEME,
    isDark: false,
    isLoading: false,
    accentColor: DEFAULT_ACCENT,
    themes: SYNTAX_THEMES,
    setTheme: () => {},
    setAccentColor: () => {},
  };
}
