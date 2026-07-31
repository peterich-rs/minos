import type { ReactNode } from "react";

import { cn } from "@/shared/lib/utils";

/**
 * Glass shell for sidebar status cards (connection / update).
 * Matches Buzz-style frosted panels sitting on gradient chrome.
 */
export function SidebarGlassCard({
  children,
  tone = "neutral",
  className,
}: {
  children: ReactNode;
  tone?: "neutral" | "danger" | "success";
  className?: string;
}) {
  return (
    <div className={cn("px-2.5 py-2", className)}>
      <div
        className={cn(
          "overflow-hidden rounded-xl border bg-surface/75 shadow-sm backdrop-blur-md",
          tone === "danger" && "border-status-failed/20",
          tone === "success" && "border-status-done/20",
          tone === "neutral" && "border-ink/8",
        )}
      >
        {children}
      </div>
    </div>
  );
}
