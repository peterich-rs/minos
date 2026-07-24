import { memo, useEffect, useState, type ReactNode } from "react";
import {
  createHighlighter,
  type Highlighter,
  type BundledLanguage,
  type ThemedToken,
} from "shiki";
import { cn } from "@/shared/lib/utils";
import {
  resolveShikiThemeName,
  type SyntaxThemeName,
} from "@/shared/theme/theme-loader";
import { useThemeOptional } from "@/shared/theme/ThemeProvider";

const MAX_LINES = 150;
const LANG_CACHE = new Set<string>();

let highlighterPromise: Promise<Highlighter> | null = null;

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighter({ themes: [], langs: [] });
  }
  return highlighterPromise;
}

function normalizeLang(className?: string): string {
  const raw =
    className
      ?.split(/\s+/)
      .find((c) => c.startsWith("language-"))
      ?.slice("language-".length) ?? "text";
  return raw.toLowerCase() || "text";
}

function tokensToNodes(lines: ThemedToken[][]): ReactNode {
  return lines.map((line, lineIdx) => (
    <span key={`l-${lineIdx}`}>
      {line.map((token, tokenIdx) => (
        <span
          key={`t-${lineIdx}-${tokenIdx}`}
          style={token.color ? { color: token.color } : undefined}
        >
          {token.content}
        </span>
      ))}
      {lineIdx < lines.length - 1 ? "\n" : null}
    </span>
  ));
}

/**
 * Shiki-highlighted fenced code block. Falls back to plain mono until ready.
 * Renders token spans (no dangerouslySetInnerHTML).
 */
export const CodeBlock = memo(function CodeBlock({
  className,
  children,
}: {
  className?: string;
  children?: React.ReactNode;
}) {
  const { themeName } = useThemeOptional();
  const code = String(children ?? "").replace(/\n$/, "");
  const lang = normalizeLang(className);
  const [tokenLines, setTokenLines] = useState<ThemedToken[][] | null>(null);

  useEffect(() => {
    let cancelled = false;
    const lines = code.split("\n");
    if (lines.length > MAX_LINES) {
      setTokenLines(null);
      return;
    }

    void (async () => {
      try {
        const highlighter = await getHighlighter();
        const shikiTheme = resolveShikiThemeName(themeName) as SyntaxThemeName;
        const loadedThemes = highlighter.getLoadedThemes();
        if (!loadedThemes.includes(shikiTheme as never)) {
          await highlighter.loadTheme(shikiTheme as never);
        }
        if (lang !== "text" && !LANG_CACHE.has(lang)) {
          try {
            await highlighter.loadLanguage(lang as BundledLanguage);
            LANG_CACHE.add(lang);
          } catch {
            /* unknown lang → plain */
          }
        }
        if (cancelled) return;
        const result = highlighter.codeToTokens(code, {
          lang: (LANG_CACHE.has(lang) ? lang : "text") as BundledLanguage,
          theme: shikiTheme,
        });
        if (!cancelled) setTokenLines(result.tokens);
      } catch {
        if (!cancelled) setTokenLines(null);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [code, lang, themeName]);

  if (tokenLines) {
    return (
      <pre className="md-pre scrollbar-thin mb-2.5 overflow-x-auto font-mono text-[12px] last:mb-0">
        <code className={cn("font-mono text-[12px] leading-relaxed", className)}>
          {tokensToNodes(tokenLines)}
        </code>
      </pre>
    );
  }

  return (
    <pre className="md-pre scrollbar-thin mb-2.5 font-mono text-[12px] last:mb-0">
      <code className={cn("font-mono text-[12px] leading-relaxed", className)}>
        {code}
      </code>
    </pre>
  );
});
