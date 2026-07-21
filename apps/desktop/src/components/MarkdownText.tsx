import { memo } from "react";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

/**
 * Markdown for conversation timeline + agent transcripts.
 *
 * `react-markdown` + `remark-gfm` for completed bodies.
 * While `streaming` is true, render plain pre-wrap text so every token does
 * not rebuild the full MDAST (dominant cost behind scroll jank during live
 * runs). One markdown pass runs when streaming ends.
 *
 * Raw HTML off by default. Keep component overrides minimal.
 */
export const MarkdownText = memo(function MarkdownText({
  text,
  className,
  streaming,
  tone = "default",
}: {
  text: string;
  className?: string;
  streaming?: boolean;
  tone?: "default" | "onDark";
}) {
  const onDark = tone === "onDark";
  // Guard wire/IPC nulls — react-markdown throws on non-string children.
  const body =
    typeof text === "string" ? text : text == null ? "" : String(text);

  const shell = cn(
    "markdown-body text-[13.5px] leading-relaxed",
    onDark ? "text-white markdown-tone-dark" : "text-ink markdown-tone-light",
    className,
  );

  // Streaming path: avoid full GFM parse on every token.
  if (streaming) {
    return (
      <div className={shell}>
        <p className="mb-0 whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
          {body}
          <span
            className={cn(
              "inline-block animate-pulse",
              onDark ? "text-white/60" : "text-ink-muted",
            )}
            aria-hidden
          >
            █
          </span>
        </p>
      </div>
    );
  }

  return (
    <div className={shell}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {body}
      </ReactMarkdown>
    </div>
  );
});

/**
 * Shared element map. Visual differences for user bubbles use parent
 * `.markdown-tone-dark` / `.markdown-tone-light` (see index.css) so we do not
 * duplicate two full component trees that drift apart.
 */
const components: Components = {
  h1: ({ children }) => (
    <h1 className="mb-1.5 mt-3 text-[15px] font-semibold first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="mb-1.5 mt-3 text-[14px] font-semibold first:mt-0">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="mb-1 mt-2.5 text-[13.5px] font-semibold first:mt-0">
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4 className="mb-1 mt-2 text-[13px] font-semibold first:mt-0">
      {children}
    </h4>
  ),
  p: ({ children }) => (
    <p className="mb-2 break-words last:mb-0 [overflow-wrap:anywhere]">
      {children}
    </p>
  ),
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="md-a underline underline-offset-2"
    >
      {children}
    </a>
  ),
  strong: ({ children }) => (
    <strong className="font-semibold">{children}</strong>
  ),
  em: ({ children }) => <em className="italic">{children}</em>,
  /**
   * Block vs inline:
   * - Fenced blocks render as <pre><code className="language-?">…
   * - Bare fences often have **no** className — still block because parent is pre.
   * - True inline is <code> without pre ancestor (react-markdown sets no special
   *   prop in v9+; use newline / language class heuristics only for styling).
   */
  code: ({ className, children }) => {
    const text = String(children ?? "");
    const hasLang = Boolean(className?.includes("language-"));
    const multiline = text.includes("\n");
    // Block code: language tag OR multiline (bare fence). Leave unstyled chip.
    if (hasLang || multiline) {
      return (
        <code className={cn("font-mono text-[12px] leading-relaxed", className)}>
          {children}
        </code>
      );
    }
    return (
      <code className="md-code-inline rounded px-1 py-0.5 font-mono text-[12px]">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="md-pre scrollbar-thin mb-2 max-w-full overflow-x-auto rounded-lg border px-3 py-2 font-mono text-[12px] leading-relaxed last:mb-0">
      {children}
    </pre>
  ),
  ul: ({ children }) => (
    <ul className="mb-2 list-disc space-y-1 pl-5 last:mb-0">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-2 list-decimal space-y-1 pl-5 last:mb-0">{children}</ol>
  ),
  li: ({ children }) => (
    <li className="break-words [overflow-wrap:anywhere]">{children}</li>
  ),
  blockquote: ({ children }) => (
    <blockquote className="md-quote mb-2 border-l-2 pl-3 last:mb-0">
      {children}
    </blockquote>
  ),
  // Keep hr a 1px rule — never a filled pill (that was confused with scrollbars).
  hr: () => <hr className="md-hr my-3 border-0 border-t" />,
  table: ({ children }) => (
    <div className="scrollbar-thin mb-2 max-w-full overflow-x-auto last:mb-0">
      <table className="w-full border-collapse text-left text-[12px]">
        {children}
      </table>
    </div>
  ),
  th: ({ children }) => (
    <th className="md-th border px-2 py-1 font-semibold">{children}</th>
  ),
  td: ({ children }) => (
    <td className="md-td border px-2 py-1 align-top">{children}</td>
  ),
};
