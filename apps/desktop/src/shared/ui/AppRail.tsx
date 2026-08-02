import { Sparkles } from "lucide-react";
import type { ReactNode } from "react";

import { shellChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/utils";

/**
 * Shared app rail chrome (Desktop Host Console + Web Cloud Console).
 * Hosts own data; this owns structure, spacing, and selected states.
 */

export type RailNavItem = {
  id: string;
  label: string;
  /** Pre-rendered icon (avoids cross-app Lucide/React type clashes). */
  icon: ReactNode;
  badge?: number;
};

export function AppRail({
  brandSubtitle,
  brandMeta,
  navItems,
  activeNavId,
  onNavSelect,
  projectsHeader,
  projects,
  footer,
  className,
}: {
  brandSubtitle?: ReactNode;
  brandMeta?: ReactNode;
  navItems: RailNavItem[];
  activeNavId: string;
  onNavSelect: (id: string) => void;
  projectsHeader?: ReactNode;
  projects: ReactNode;
  footer?: ReactNode;
  className?: string;
}) {
  return (
    <aside
      className={cn(
        "relative z-10 flex h-full min-h-0 shrink-0 flex-col bg-transparent",
        shellChrome.sidebarWidth,
        className,
      )}
      data-testid="app-sidebar"
    >
      <div className="flex shrink-0 items-center gap-2.5 px-3.5 py-3.5">
        <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-ink text-surface shadow-sm">
          <Sparkles className="h-4 w-4" strokeWidth={2.2} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold tracking-tight text-ink">
            Minos
          </div>
          {brandSubtitle ? (
            <div className="mt-0.5 min-w-0 text-2xs text-ink-secondary/80">
              {brandSubtitle}
            </div>
          ) : null}
          {brandMeta}
        </div>
      </div>

      <nav className="shrink-0 space-y-0.5 px-2 pb-2">
        {navItems.map((item) => {
          const active = activeNavId === item.id;
          const badge = item.badge ?? 0;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onNavSelect(item.id)}
              className={cn(
                "relative flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors duration-150",
                active
                  ? "bg-primary font-medium text-white shadow-sm"
                  : "text-ink-secondary hover:bg-ink/5 hover:text-ink",
              )}
            >
              <span className="relative z-[1] flex h-4 w-4 shrink-0 items-center justify-center opacity-90 [&>svg]:h-4 [&>svg]:w-4">
                {item.icon}
              </span>
              <span className="relative z-[1] flex-1">{item.label}</span>
              {badge > 0 ? (
                <span
                  className={cn(
                    "relative z-[1] inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-3xs font-semibold tabular-nums leading-none",
                    active
                      ? "bg-white/20 text-white"
                      : "bg-status-approval text-white",
                  )}
                >
                  {badge}
                </span>
              ) : null}
            </button>
          );
        })}
      </nav>

      <div className="mx-3 h-px shrink-0 bg-ink/8" />

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {projectsHeader}
        <div className="scrollbar-thin min-h-0 flex-1 space-y-0.5 overflow-x-hidden overflow-y-auto px-2 pb-3">
          {projects}
        </div>
      </div>

      {footer ? <div className="mt-auto shrink-0">{footer}</div> : null}
    </aside>
  );
}

export function AppRailProjectsHeader({
  action,
}: {
  action?: ReactNode;
}) {
  return (
    <div className="mt-1 flex shrink-0 items-center justify-between px-3.5 pb-1.5 pt-2">
      <span className="text-2xs font-semibold uppercase tracking-[0.06em] text-ink-muted">
        Projects
      </span>
      {action}
    </div>
  );
}

export function AppRailProjectRow({
  name,
  path,
  active,
  attention = 0,
  running = false,
  hostLabel,
  leading,
  onClick,
}: {
  name: string;
  path: string;
  active: boolean;
  attention?: number;
  running?: boolean;
  hostLabel?: string | null;
  leading?: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)] items-start gap-x-2 gap-y-0.5 rounded-lg px-2 py-2 text-left transition-colors duration-150",
        active
          ? "bg-surface/90 shadow-sm ring-1 ring-ink/10 backdrop-blur-sm"
          : "hover:bg-surface/50",
      )}
    >
      {leading}
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            className="min-w-0 flex-1 truncate text-sm font-medium leading-snug text-ink"
            title={name}
          >
            {name}
          </span>
          {running ? (
            <span
              className="h-1.5 w-1.5 shrink-0 rounded-full bg-status-running"
              title="Agents running"
            />
          ) : null}
          {attention > 0 ? (
            <span className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-status-approval px-1 text-3xs font-semibold tabular-nums leading-none text-white">
              {attention}
            </span>
          ) : null}
        </div>
        <div
          className="truncate font-mono text-2xs leading-snug text-ink-muted"
          title={path}
        >
          {path.replace(/^~\//, "")}
        </div>
        {hostLabel ? (
          <div className="mt-0.5 truncate text-3xs text-ink-muted/90">
            {hostLabel}
          </div>
        ) : null}
      </div>
    </button>
  );
}

export function AppRailAccountFooter({
  email,
  statusLabel = "Online",
  onSignOut,
  extra,
}: {
  email: string;
  statusLabel?: string;
  onSignOut?: () => void;
  extra?: ReactNode;
}) {
  return (
    <div className="shrink-0 space-y-2 border-t border-ink/8 p-2.5">
      {extra}
      <div className="rounded-xl border border-ink/6 bg-surface/80 px-3 py-2.5 shadow-sm backdrop-blur-md">
        <div className="text-2xs font-medium text-ink-muted">Signed in</div>
        <div className="mt-0.5 truncate text-sm font-medium text-ink">
          {email || "—"}
        </div>
        <div className="mt-1.5 flex items-center gap-1.5 text-2xs text-ink-muted">
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-status-done opacity-40 motion-reduce:animate-none" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-status-done" />
          </span>
          {statusLabel}
        </div>
      </div>
      {onSignOut ? (
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center justify-center gap-2 rounded-lg px-3 py-2 text-sm text-ink-secondary transition-colors hover:bg-ink/5 hover:text-ink"
        >
          Sign out
        </button>
      ) : null}
    </div>
  );
}
