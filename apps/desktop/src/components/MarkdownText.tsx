import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Lightweight markdown for agent transcripts (TUI parity, no heavy deps).
 * Supports: fenced code, inline code, **bold**, *italic*, paragraphs, bare links.
 */
export function MarkdownText({
  text,
  className,
  streaming,
}: {
  text: string;
  className?: string;
  streaming?: boolean;
}) {
  const blocks = splitBlocks(text);
  return (
    <div
      className={cn(
        "markdown-body space-y-2 text-[13.5px] leading-relaxed text-ink",
        className,
      )}
    >
      {blocks.map((block, i) => {
        if (block.type === "code") {
          return (
            <pre
              key={i}
              className="overflow-x-auto rounded-lg border border-ink/10 bg-surface-muted/70 px-3 py-2 font-mono text-[12px] leading-relaxed text-ink-secondary"
            >
              <code>{block.content}</code>
            </pre>
          );
        }
        return (
          <p key={i} className="whitespace-pre-wrap break-words">
            {renderInline(block.content)}
            {streaming && i === blocks.length - 1 ? (
              <span className="ml-0.5 inline-block animate-pulse text-ink-muted">
                █
              </span>
            ) : null}
          </p>
        );
      })}
      {streaming && blocks.length === 0 ? (
        <span className="inline-block animate-pulse text-ink-muted">█</span>
      ) : null}
    </div>
  );
}

type Block =
  | { type: "text"; content: string }
  | { type: "code"; content: string; lang?: string };

function splitBlocks(source: string): Block[] {
  const blocks: Block[] = [];
  const re = /```([^\n`]*)\n?([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    if (m.index > last) {
      const chunk = source.slice(last, m.index).trimEnd();
      if (chunk) blocks.push({ type: "text", content: chunk });
    }
    blocks.push({
      type: "code",
      lang: m[1]?.trim() || undefined,
      content: (m[2] ?? "").replace(/\n$/, ""),
    });
    last = m.index + m[0].length;
  }
  if (last < source.length) {
    const chunk = source.slice(last);
    if (chunk.trim()) blocks.push({ type: "text", content: chunk });
  }
  if (blocks.length === 0 && source) {
    blocks.push({ type: "text", content: source });
  }
  return blocks;
}

function renderInline(text: string): ReactNode[] {
  // Order: code, bold, italic, bare urls
  const parts: ReactNode[] = [];
  const re =
    /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|https?:\/\/[^\s<]+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      parts.push(text.slice(last, m.index));
    }
    const token = m[0]!;
    if (token.startsWith("`")) {
      parts.push(
        <code
          key={key++}
          className="rounded bg-surface-muted px-1 py-0.5 font-mono text-[12px] text-ink-secondary"
        >
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith("**")) {
      parts.push(
        <strong key={key++} className="font-semibold">
          {token.slice(2, -2)}
        </strong>,
      );
    } else if (token.startsWith("*")) {
      parts.push(
        <em key={key++} className="italic">
          {token.slice(1, -1)}
        </em>,
      );
    } else if (token.startsWith("http")) {
      parts.push(
        <a
          key={key++}
          href={token}
          target="_blank"
          rel="noreferrer"
          className="text-amber-800 underline decoration-amber-800/30 underline-offset-2 hover:decoration-amber-800"
        >
          {token}
        </a>,
      );
    } else {
      parts.push(token);
    }
    last = m.index + token.length;
  }
  if (last < text.length) {
    parts.push(text.slice(last));
  }
  return parts;
}
