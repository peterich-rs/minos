import type { CSSProperties, ReactNode } from "react";

import { chromeCssVarDefaults } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/utils";

/**
 * Buzz-inspired app chrome shared by Desktop Host Console and Web Cloud Console.
 *
 * Structure:
 *   [full-viewport gradient + grain]
 *   [sidebar on gradient] [floating content surface]
 */
export function ShellFrame({
  sidebar,
  children,
  className,
}: {
  sidebar: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "relative flex h-full w-full min-h-0 overflow-hidden",
        className,
      )}
      style={chromeCssVarDefaults as CSSProperties}
      data-minos-shell
    >
      <div className="minos-theme-gradient" aria-hidden />
      <div className="minos-theme-grain" aria-hidden />

      <div className="relative z-10 flex h-full w-full min-h-0 min-w-0 pb-2 pl-px pr-2 pt-px">
        {/* h-full so sidebar flex footer can pin; overflow clip selection chrome. */}
        <div className="relative z-20 flex h-full min-h-0 shrink-0 flex-col overflow-hidden">
          {sidebar}
        </div>
        <div className="minos-content-surface relative z-10 ml-1 min-w-0">
          {children}
        </div>
      </div>
    </div>
  );
}
