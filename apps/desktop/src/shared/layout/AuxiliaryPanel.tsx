import type { ReactNode } from "react";
import { cn } from "@/shared/lib/utils";
import {
  AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
  AUXILIARY_PANEL_MIN_WIDTH_PX,
  shellChrome,
} from "@/shared/layout/chromeLayout";

export type AuxiliaryPanelLayout = "split" | "rail" | "overlay";

type AuxiliaryPanelProps = {
  children: ReactNode;
  /** Optional footer strip (border-t). */
  footer?: ReactNode;
  /** Header row (border-b). Usually title + close. */
  header?: ReactNode;
  /**
   * - `split` — fill parent (resizable Panel child)
   * - `rail` — fixed-width column in normal flow
   * - `overlay` — fixed right drawer + dimmed backdrop
   */
  layout?: AuxiliaryPanelLayout;
  onClose: () => void;
  className?: string;
  testId?: string;
  /** Rail/overlay width in px (split ignores and fills parent). */
  widthPx?: number;
  /** When false, skip slide-in animation (parent already animates). */
  enterMotion?: boolean;
};

/**
 * Right-side auxiliary panel shell (inspector / detail).
 * Split fills a resizable host; rail is in-flow; overlay floats with backdrop.
 */
export function AuxiliaryPanel({
  children,
  className,
  enterMotion = true,
  footer,
  header,
  layout = "rail",
  onClose,
  testId,
  widthPx = AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
}: AuxiliaryPanelProps) {
  const isOverlay = layout === "overlay";
  const isSplit = layout === "split";

  const panel = (
    <aside
      data-testid={testId}
      data-layout={layout}
      className={cn(
        "relative flex h-full min-h-0 flex-col overflow-hidden border-l border-ink/5 bg-surface",
        isSplit && "w-full min-w-0",
        layout === "rail" && shellChrome.auxiliaryWidth,
        isOverlay &&
          "fixed bottom-0 right-0 top-0 z-40 h-auto max-w-[min(100vw-2rem,420px)] shadow-2xl",
        enterMotion && !isSplit && "minos-side-panel-enter",
        className,
      )}
      style={
        isOverlay
          ? {
              width: `min(${Math.max(widthPx, AUXILIARY_PANEL_MIN_WIDTH_PX)}px, calc(100vw - 2rem))`,
            }
          : layout === "rail"
            ? {
                // Prefer explicit width when caller overrides default CSS var.
                width: `${Math.max(widthPx, AUXILIARY_PANEL_MIN_WIDTH_PX)}px`,
                minWidth: `${AUXILIARY_PANEL_MIN_WIDTH_PX}px`,
                maxWidth: "min(420px, 90vw)",
              }
            : undefined
      }
    >
      {header ? <div className="shrink-0">{header}</div> : null}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {children}
      </div>
      {footer ? (
        <div className="shrink-0 border-t border-ink/5">{footer}</div>
      ) : null}
    </aside>
  );

  if (!isOverlay) {
    return panel;
  }

  return (
    <>
      <button
        type="button"
        aria-label="Close panel"
        className="fixed inset-0 z-30 cursor-default bg-ink/20 motion-reduce:transition-none"
        onClick={onClose}
      />
      {panel}
    </>
  );
}
