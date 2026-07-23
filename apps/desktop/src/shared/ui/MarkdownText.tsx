import { memo, type ReactNode } from "react";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/shared/lib/utils";
import { CodeBlock } from "@/shared/ui/markdown/CodeBlock";

/**
 * Markdown for conversation timeline + agent transcripts.
 *
 * `react-markdown` + `remark-gfm` for completed bodies.
 * While `streaming` is true, render plain pre-wrap text so every token does
 * not rebuild the full MDAST (dominant cost behind scroll jank during live
 * runs). One markdown pass runs when streaming ends.
 *
 * Raw HTML off by default. Visual tones live in index.css
 * (`.markdown-tone-light` / `.markdown-tone-dark`).
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
 * Shared element map. Tone-dependent colors use `.md-*` classes + CSS vars
 * so light/dark bubble chrome does not drift across two component trees.
 */
const components: Components = {
  h1: ({ children }) => (
    <h1 className="mb-2 mt-3.5 text-[15px] font-semibold leading-snug first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="mb-1.5 mt-3 text-[14px] font-semibold leading-snug first:mt-0">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="mb-1 mt-2.5 text-[13.5px] font-semibold leading-snug first:mt-0">
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4 className="mb-1 mt-2 text-[13px] font-semibold leading-snug first:mt-0">
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
   * - True inline is <code> without pre ancestor (react-markdown v9+).
   */
  code: ({ className, children }) => {
    const text = String(children ?? "");
    const hasLang = Boolean(className?.includes("language-"));
    const multiline = text.includes("\n");
    // Inline only — fenced blocks go through `pre` → CodeBlock (Shiki).
    if (hasLang || multiline) {
      return (
        <code className={cn("font-mono text-[12px] leading-relaxed", className)}>
          {children}
        </code>
      );
    }
    return (
      <code className="md-code-inline font-mono text-[12px]">{children}</code>
    );
  },
  pre: ({ children }) => {
    // react-markdown nests <code className="language-…"> inside <pre>.
    const child = Array.isArray(children) ? children[0] : children;
    if (
      child &&
      typeof child === "object" &&
      "props" in child &&
      child.props &&
      typeof child.props === "object"
    ) {
      const props = child.props as {
        className?: string;
        children?: ReactNode;
      };
      return (
        <CodeBlock className={props.className}>{props.children}</CodeBlock>
      );
    }
    return (
      <pre className="md-pre scrollbar-thin mb-2.5 font-mono text-[12px] last:mb-0">
        {children}
      </pre>
    );
  },
  ul: ({ children }) => (
    <ul className="mb-2 space-y-1 pl-5 last:mb-0">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-2 space-y-1 pl-5 last:mb-0">{children}</ol>
  ),
  li: ({ children }) => (
    <li className="break-words [overflow-wrap:anywhere] [&>ul]:mt-1 [&>ol]:mt-1">
      {children}
    </li>
  ),
  blockquote: ({ children }) => (
    <blockquote className="md-quote last:mb-0">{children}</blockquote>
  ),
  // Keep hr a 1px rule — never a filled pill (confused with scrollbars).
  hr: () => <hr className="md-hr" />,
  table: ({ children }) => (
    <div className="md-table-wrap scrollbar-thin mb-2.5 max-w-full overflow-x-auto last:mb-0">
      <table className="w-full text-left text-[12px] leading-snug">
        {children}
      </table>
    </div>
  ),
  th: ({ children }) => (
    <th className="md-th align-top">{children}</th>
  ),
  td: ({ children }) => (
    <td className="md-td align-top">{children}</td>
  ),
};
