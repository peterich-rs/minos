import { MessageSquare, Plus, Search } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/shared/lib/utils";

/**
 * Shared Work surface chrome — Project header, conversation rail, timeline shell.
 * Desktop and Web both compose this so layout/classnames stay identical.
 */

export function WorkSurface({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-canvas-soft/40">
      {children}
    </div>
  );
}

export type WorkViewTab = {
  id: string;
  label: string;
  /** Pre-rendered icon node (avoids cross-app Lucide/React type clashes). */
  icon?: ReactNode;
};

export function WorkProjectHeader({
  projectName,
  projectPath,
  tabs,
  activeTabId,
  onTabSelect,
  meta,
  onSearch,
  onNew,
  searchDisabled,
  newDisabled,
  newLabel = "New conversation",
}: {
  projectName: string;
  projectPath: string;
  tabs: WorkViewTab[];
  activeTabId: string;
  onTabSelect?: (id: string) => void;
  meta?: ReactNode;
  onSearch?: () => void;
  onNew?: () => void;
  searchDisabled?: boolean;
  newDisabled?: boolean;
  newLabel?: string;
}) {
  return (
    <header className="shrink-0 border-b border-ink/6 bg-surface/95 px-4 pt-3.5 backdrop-blur-sm sm:px-5">
      <div className="flex min-w-0 items-center gap-2 sm:gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-2 sm:gap-3">
          <h1
            className="max-w-[40%] shrink truncate text-lg font-semibold tracking-tight text-ink sm:max-w-[32%]"
            title={projectName}
          >
            {projectName}
          </h1>
          <span className="hidden h-4 w-px shrink-0 bg-ink/10 sm:block" />
          <p
            className="min-w-0 flex-1 truncate font-mono text-2xs text-ink-muted sm:text-xs"
            title={projectPath}
          >
            {projectPath}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5 sm:gap-2">
          <button
            type="button"
            disabled={searchDisabled}
            onClick={onSearch}
            className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-ink/10 bg-surface px-2.5 text-xs font-medium text-ink-secondary shadow-sm transition-colors hover:bg-surface-hover disabled:opacity-45 sm:px-3"
          >
            <Search className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">Search</span>
          </button>
          <button
            type="button"
            disabled={newDisabled}
            onClick={onNew}
            className="inline-flex h-9 items-center gap-1.5 rounded-lg bg-primary px-2.5 text-xs font-semibold text-white shadow-sm transition-opacity hover:opacity-90 disabled:opacity-45 sm:px-3"
          >
            <Plus className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{newLabel}</span>
            <span className="sm:hidden">New</span>
          </button>
        </div>
      </div>

      <div className="mt-2.5 flex min-w-0 items-center gap-0.5">
        {tabs.map((tab) => {
          const active = activeTabId === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => onTabSelect?.(tab.id)}
              className={cn(
                "relative inline-flex h-9 shrink-0 items-center gap-1.5 px-2.5 text-sm font-medium transition-colors sm:px-3",
                active ? "text-ink" : "text-ink-muted hover:text-ink-secondary",
              )}
            >
              {tab.icon}
              {tab.label}
              {active ? (
                <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-primary" />
              ) : null}
            </button>
          );
        })}
        {meta ? (
          <div className="ml-auto flex min-w-0 items-center gap-2 pb-0.5 text-2xs text-ink-muted sm:gap-3 sm:text-xs">
            {meta}
          </div>
        ) : null}
      </div>
    </header>
  );
}

export function WorkConversationRail({
  title = "Conversations",
  subtitle,
  actions,
  children,
  /** Fill a resizable panel (Desktop) instead of fixed rail width. */
  fill = false,
  className,
}: {
  title?: string;
  subtitle?: string;
  actions?: ReactNode;
  children: ReactNode;
  fill?: boolean;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "flex h-full min-h-0 flex-col overflow-hidden border-r border-ink/6 bg-surface",
        fill
          ? "w-full min-w-0"
          : "w-[min(280px,34vw)] min-w-[220px] max-w-[340px] shrink-0",
        className,
      )}
    >
      <div className="flex shrink-0 items-center justify-between gap-1 border-b border-ink/6 px-3 py-3">
        <div className="min-w-0 pl-1">
          <div className="text-sm font-semibold tracking-tight text-ink">
            {title}
          </div>
          {subtitle ? (
            <div className="truncate text-2xs text-ink-muted">{subtitle}</div>
          ) : null}
        </div>
        {actions}
      </div>
      {/* overflow-hidden so VirtualizedList owns scroll; wrap plain lists in overflow-y-auto */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden p-2">
        {children}
      </div>
    </section>
  );
}

export function WorkConversationRow({
  title,
  /** Right-aligned time / status next to title (Desktop list). */
  titleTrailing,
  preview,
  meta,
  selected,
  onSelect,
  leading,
}: {
  title: string;
  titleTrailing?: ReactNode;
  preview?: string;
  meta?: ReactNode;
  selected: boolean;
  onSelect: () => void;
  /** Defaults to MessageSquare; override for host-specific icons. */
  leading?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex w-full gap-2.5 rounded-xl px-3 py-2.5 text-left transition-colors duration-150",
        selected
          ? "bg-primary/10 shadow-sm ring-1 ring-primary/25"
          : "hover:bg-surface-hover",
      )}
    >
      {leading ?? (
        <MessageSquare
          className={cn(
            "mt-0.5 h-3.5 w-3.5 shrink-0",
            selected ? "text-primary" : "text-ink-muted",
          )}
        />
      )}
      <div className="min-w-0 flex-1">
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-2">
          <span
            className="truncate text-sm font-semibold leading-snug text-ink"
            title={title}
          >
            {title}
          </span>
          {titleTrailing}
        </div>
        {preview ? (
          <p
            className="mt-0.5 line-clamp-2 text-xs leading-snug text-ink-muted"
            title={preview}
          >
            {preview}
          </p>
        ) : null}
        {meta ? (
          <div className="mt-1.5 flex flex-wrap items-center gap-1">{meta}</div>
        ) : null}
      </div>
    </button>
  );
}

export function WorkTimelineShell({
  header,
  children,
  composer,
}: {
  header: ReactNode;
  children: ReactNode;
  composer: ReactNode;
}) {
  return (
    <section className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-canvas-soft/25">
      {header}
      <div className="relative min-h-0 flex-1">{children}</div>
      {composer}
    </section>
  );
}

export function WorkTimelineHeader({
  title,
  subtitle,
  trailing,
}: {
  title: string;
  subtitle?: ReactNode;
  trailing?: ReactNode;
}) {
  return (
    <header className="flex shrink-0 items-center justify-between gap-3 border-b border-ink/6 bg-surface/90 px-4 py-3.5 backdrop-blur-sm sm:px-5">
      <div className="min-w-0 flex-1">
        <h2 className="truncate text-base font-semibold tracking-tight text-ink">
          {title}
        </h2>
        {subtitle ? (
          <div className="mt-1 text-2xs text-ink-muted">{subtitle}</div>
        ) : null}
      </div>
      {trailing}
    </header>
  );
}
