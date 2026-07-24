import { memo, useCallback, useEffect, useState } from "react";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  FilePenLine,
  FileText,
  FolderOpen,
  Globe,
  Search,
  ShieldAlert,
  Sparkles,
  Terminal,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { DiffView } from "@/shared/ui/DiffView";
import { ReadView, shouldUseReadView } from "@/shared/ui/ReadView";
import { MarkdownText } from "@/shared/ui/MarkdownText";
import { cn } from "@/shared/lib/utils";
import type { TranscriptItem } from "@/shared/lib/daemon";
import {
  buildToolHeader,
  collapsedThinkingSummary,
  displayToolDetail,
  isDiffLike,
  type ToolKind,
} from "@/shared/lib/tool-present";
import { ApprovalModal } from "./ApprovalModal";
import type { UserAction } from "../lib/user-action";

/** Small tool-kind glyphs for session transcript rows (Grok-style scanability). */
const TOOL_KIND_ICON: Record<ToolKind, LucideIcon> = {
  read: FileText,
  edit: FilePenLine,
  execute: Terminal,
  search: Search,
  list: FolderOpen,
  web_fetch: Globe,
  web_search: Globe,
  skill: Sparkles,
  other: Wrench,
};

/**
 * Grok / TUI AgentDetail-style transcript row (not messenger bubbles).
 * Memoized so stream/store ticks only re-render the active row + changed props.
 */
export const TranscriptItemView = memo(function TranscriptItemView({
  item,
  streaming,
  onUserAction,
  approving,
}: {
  item: TranscriptItem;
  streaming?: boolean;
  onUserAction?: (
    item: TranscriptItem,
    action: UserAction,
  ) => void | Promise<void>;
  approving?: boolean;
}) {
  const [open, setOpen] = useState(Boolean(streaming));
  const [planOpen, setPlanOpen] = useState(false);

  const runAction = useCallback(
    (action: UserAction) => onUserAction?.(item, action),
    [onUserAction, item],
  );

  // Stream start re-opens thinking (TUI default expand while streaming).
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  if (item.kind === "approval" || item.kind === "question") {
    const isPlan = item.approvalMethod === "x.ai/exit_plan_mode";
    const isQuestion = item.kind === "question";
    // No requestId → already answered (history demote / local resolve). Do not
    // re-show interactive plan/permission chrome for a finished reverse-request.
    if (!item.requestId) {
      return (
        <div className="text-[12px] text-ink-muted">
          {item.title ? `${item.title} · ` : null}
          {item.text}
        </div>
      );
    }
    return (
      <>
        <div className="rounded-xl border border-rose-200/80 bg-rose-50/80 p-3">
          <div className="flex items-start gap-2.5">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-rose-600" />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-rose-900">
                {item.title ??
                  (isQuestion ? "Question" : "Approval required")}
              </div>
              <p className="mt-1 whitespace-pre-wrap text-[12.5px] leading-snug text-rose-900/80">
                {item.text}
              </p>
              {isQuestion && item.options && item.options.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {item.options.map((opt) => (
                    <button
                      key={opt.label}
                      type="button"
                      disabled={approving}
                      onClick={() => {
                        void runAction({
                          type: "decision",
                          decision: opt.label,
                        });
                      }}
                      className="rounded-lg border border-rose-300/80 bg-white px-2.5 py-1 text-[12px] font-medium text-rose-900 hover:bg-rose-50 disabled:opacity-50"
                    >
                      {opt.label}
                    </button>
                  ))}
                  <button
                    type="button"
                    disabled={approving}
                    onClick={() => {
                      void runAction({ type: "cancel" });
                    }}
                    className="rounded-lg px-2.5 py-1 text-[12px] font-medium text-rose-700/80 hover:bg-rose-100/60 disabled:opacity-50"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <div className="mt-2.5 flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setPlanOpen(true)}
                    className="rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white hover:bg-ink/90"
                  >
                    {isPlan ? "View plan" : "View details"}
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
        <ApprovalModal
          item={item}
          isPlan={isPlan}
          open={planOpen}
          approving={approving}
          onClose={() => setPlanOpen(false)}
          onUserAction={runAction}
        />
      </>
    );
  }

  if (item.kind === "user") {
    return (
      <div className="text-[13.5px] leading-relaxed text-ink">
        <span className="select-none text-ink-muted">❯ </span>
        <span className="whitespace-pre-wrap break-words">{item.text}</span>
        {streaming ? (
          <span className="ml-0.5 inline-block animate-pulse text-ink-muted">
            █
          </span>
        ) : null}
      </div>
    );
  }

  if (item.kind === "assistant" || item.kind === "text") {
    return <MarkdownText text={item.text} streaming={streaming} />;
  }

  if (item.kind === "reasoning") {
    const header = streaming ? "Thinking…" : "Thought";
    const preview = collapsedThinkingSummary(item.text, 100);
    return (
      <div className="text-[12.5px] leading-relaxed">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex w-full items-center gap-1.5 text-left text-ink-secondary hover:text-ink"
        >
          {open ? (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          )}
          <span className="font-medium text-ink-muted">{header}</span>
          {!open && preview ? (
            <span className="min-w-0 truncate text-ink-muted/80">{preview}</span>
          ) : null}
        </button>
        {open ? (
          <div className="mt-1 space-y-0.5 border-l-2 border-ink/10 pl-3 text-ink-secondary">
            {item.text.split("\n").map((line, i) => (
              <div key={i} className="flex gap-2">
                <span className="select-none text-ink-muted/50">│</span>
                <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
                  {line || " "}
                </span>
              </div>
            ))}
            {streaming ? (
              <div className="flex gap-2">
                <span className="select-none text-ink-muted/50">│</span>
                <span className="animate-pulse text-ink-muted">█</span>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    );
  }

  // OpenCode / Codex subagent card (TUI SubagentCall parity).
  if (item.kind === "subagent") {
    const running = /\bRunning\b/i.test(item.text) || /\brunning\b/.test(item.text);
    const failed = /\bfailed\b/i.test(item.text) || /\binterrupted\b/i.test(item.text);
    const desc = (item.detail ?? "").trim();
    return (
      <div className="text-[12.5px] leading-snug">
        <div className="flex w-full max-w-full items-center gap-1.5">
          <span className="inline-block w-3 shrink-0" />
          <Bot
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              failed ? "text-rose-600" : "text-ink-muted",
            )}
            strokeWidth={1.8}
            aria-hidden
          />
          <span
            className={cn(
              "shrink-0 font-medium",
              failed ? "text-rose-700" : "text-ink-secondary",
            )}
          >
            {item.text.split(/\s+/)[0] ?? (running ? "Running" : "Ran")}
          </span>
          <span
            className={cn(
              "min-w-0 truncate font-mono text-[12px]",
              failed ? "text-rose-800/90" : "text-ink",
            )}
            title={item.text}
          >
            {item.text.replace(/^(Running|Ran)\s+/i, "")}
          </span>
          {running ? (
            <span className="shrink-0 text-ink-muted">…</span>
          ) : null}
        </div>
        {desc ? (
          <p className="mt-0.5 pl-8 text-[12px] text-ink-muted line-clamp-2">
            {desc}
          </p>
        ) : null}
      </div>
    );
  }

  if (
    item.kind === "tool" ||
    item.kind === "tool_result" ||
    item.kind === "tool_error"
  ) {
    const header = buildToolHeader({
      toolName: item.title ?? "tool",
      target: item.text,
      kind: item.kind,
      detail: item.detail,
    });
    // Strip SGR color codes from bash/CLI tool bodies (Grok ACP raw bytes).
    const detail = displayToolDetail(item.detail).trim();
    const expandable = Boolean(detail);
    // Only real patches (not tool-args JSON) use DiffView.
    const showDiff = detail.length > 0 && isDiffLike(detail);
    // Grok read_file emits `N→content` markers for the model; render as gutter.
    const showRead = shouldUseReadView({
      toolName: item.title ?? "tool",
      detail,
      isDiff: showDiff,
    });
    const KindIcon = TOOL_KIND_ICON[header.toolKind];
    return (
      <div className="text-[12.5px] leading-snug">
        <button
          type="button"
          disabled={!expandable}
          onClick={() => expandable && setOpen((v) => !v)}
          className={cn(
            "flex w-full max-w-full items-center gap-1.5 text-left",
            expandable ? "cursor-pointer hover:opacity-90" : "cursor-default",
          )}
        >
          {expandable ? (
            open ? (
              <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
            ) : (
              <ChevronRight className="h-3 w-3 shrink-0 text-ink-muted" />
            )
          ) : (
            <span className="inline-block w-3 shrink-0" />
          )}
          <KindIcon
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              header.failed ? "text-rose-600" : "text-ink-muted",
            )}
            strokeWidth={1.8}
            aria-hidden
          />
          <span
            className={cn(
              "shrink-0 font-medium",
              header.failed ? "text-rose-700" : "text-ink-secondary",
            )}
          >
            {header.verb}
          </span>
          <span
            className={cn(
              "min-w-0 truncate font-mono text-[12px]",
              header.failed ? "text-rose-800/90" : "text-ink",
            )}
            title={header.targetFull}
          >
            {header.target}
          </span>
          {header.running ? (
            <span className="shrink-0 text-ink-muted">…</span>
          ) : null}
          {header.failed ? (
            <span className="shrink-0 text-rose-600">failed</span>
          ) : null}
          {header.diffstat && !header.running && !header.failed ? (
            <span className="shrink-0 tabular-nums">
              <span className="text-emerald-700">+{header.diffstat.add}</span>
              <span className="text-ink-muted">/</span>
              <span className="text-rose-600">-{header.diffstat.del}</span>
            </span>
          ) : null}
        </button>
        {open && detail ? (
          showDiff ? (
            <DiffView text={detail} />
          ) : showRead ? (
            <ReadView text={detail} />
          ) : (
            <pre className="mt-1 max-h-72 overflow-auto rounded-lg border border-ink/5 bg-surface-muted/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-ink-secondary whitespace-pre-wrap">
              {detail}
            </pre>
          )
        ) : null}
      </div>
    );
  }

  if (item.kind === "error") {
    return (
      <div className="rounded-lg border border-rose-200/80 bg-rose-50/70 px-3 py-2 text-[13px] text-rose-900">
        {item.text}
      </div>
    );
  }

  if (item.kind === "status" || item.kind === "system") {
    return <div className="text-[12px] text-ink-muted">{item.text}</div>;
  }

  return (
    <div className="text-[11px] text-ink-muted">
      {item.title ?? item.kind}
      {item.text ? ` · ${item.text.slice(0, 120)}` : ""}
    </div>
  );
});
