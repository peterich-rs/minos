import type { ReactNode } from "react";

import { cn } from "@/shared/lib/utils";

/**
 * Slack/Buzz-style message row shell shared by Desktop transcript + Web mock.
 * Full-width, left-aligned for every author — no left/right bubble split.
 */
export function MessageChrome({
  groupedWithPrevious = false,
  animateIn = false,
  avatar,
  header,
  children,
  className,
  messageId,
}: {
  groupedWithPrevious?: boolean;
  /** Play enter animation (new live rows only). */
  animateIn?: boolean;
  avatar: ReactNode;
  header?: ReactNode;
  children: ReactNode;
  className?: string;
  messageId?: string;
}) {
  const enterClass = animateIn
    ? groupedWithPrevious
      ? "animate-message-in-grouped motion-reduce:animate-none"
      : "animate-message-in motion-reduce:animate-none"
    : undefined;

  return (
    <article
      className={cn(
        "group/message relative z-10 flex gap-2.5 rounded-xl px-2.5 py-1.5 transition-colors duration-150",
        "mx-1 hover:bg-ink/[0.04] focus-within:bg-ink/[0.04]",
        groupedWithPrevious ? "items-center -mt-0.5" : "items-start",
        enterClass,
        className,
      )}
      data-message-id={messageId}
      data-testid="message-row"
    >
      {avatar}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {header}
        {children}
      </div>
    </article>
  );
}

export function MessageAvatarGutter({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex w-9 shrink-0 items-start justify-center pt-0.5",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function MessageSystemChrome({
  children,
  animateIn,
  className,
}: {
  children: ReactNode;
  animateIn?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "mx-auto max-w-md rounded-xl bg-surface-muted px-3 py-2 text-center text-xs text-ink-muted",
        animateIn && "animate-message-in motion-reduce:animate-none",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function MessageAuthorLine({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "mb-0.5 flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 text-sm font-semibold text-ink",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function MessageBody({
  children,
  dimmed,
  grouped,
  className,
}: {
  children: ReactNode;
  dimmed?: boolean;
  grouped?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "relative max-w-full text-sm leading-relaxed text-ink",
        grouped ? "mt-0" : "-mt-0.5",
        dimmed && "opacity-70",
        className,
      )}
    >
      {children}
    </div>
  );
}
