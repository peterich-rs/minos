/** Conversation priority / progress / board column helpers. */

import type {
  ConversationBoardColumn,
  ConversationPriority,
  ConversationProgress,
} from "@/lib/mock-data";

export const PRIORITY_CYCLE: Array<ConversationPriority | null> = [
  null,
  "high",
  "medium",
  "low",
];

export const PROGRESS_CYCLE: ConversationProgress[] = [
  "todo",
  "in_progress",
  "in_review",
  "done",
];

/** Conversation list progress filter; `all` shows every conversation. */
export type ConversationProgressFilter = "all" | ConversationProgress;

/** User-facing list filters only — no `in_review` (not a product status). */
export const PROGRESS_FILTER_OPTIONS: Array<{
  value: ConversationProgressFilter;
  label: string;
}> = [
  { value: "all", label: "All" },
  { value: "todo", label: "To do" },
  { value: "in_progress", label: "In progress" },
  { value: "done", label: "Done" },
];

export function progressFilterLabel(
  filter: ConversationProgressFilter,
): string {
  return (
    PROGRESS_FILTER_OPTIONS.find((o) => o.value === filter)?.label ?? "All"
  );
}

/**
 * Missing progress is treated as `todo` (same as parseProgress).
 * Legacy `in_review` rows (if any) fold into the In progress filter.
 */
export function matchesProgressFilter(
  progress: ConversationProgress | undefined,
  filter: ConversationProgressFilter,
): boolean {
  if (filter === "all") return true;
  const p = progress ?? "todo";
  if (filter === "in_progress") {
    return p === "in_progress" || p === "in_review";
  }
  return p === filter;
}

export function parsePriority(
  value: string | null | undefined,
): ConversationPriority | undefined {
  if (value === "high" || value === "medium" || value === "low") {
    return value;
  }
  return undefined;
}

export function parseProgress(
  value: string | null | undefined,
): ConversationProgress {
  if (
    value === "todo" ||
    value === "in_progress" ||
    value === "in_review" ||
    value === "done"
  ) {
    return value;
  }
  return "todo";
}

/**
 * Board column derivation:
 * - done progress always stays in Done (even if sessions still running)
 * - suspended / approval sessions → Needs you (progress stays in_progress-ish)
 * - running sessions or active progress → Running
 * - else Backlog
 *
 * Needs you is runtime attention, not progress=`in_review`.
 */
export function deriveBoardColumn(input: {
  progress: ConversationProgress | undefined;
  runningCount: number;
  approvalCount: number;
}): ConversationBoardColumn {
  const progress = input.progress ?? "todo";
  if (progress === "done") return "done";
  if (input.approvalCount > 0) return "needs_you";
  if (
    input.runningCount > 0 ||
    progress === "in_progress" ||
    progress === "in_review"
  ) {
    return "running";
  }
  return "backlog";
}

/** Map a board move to the progress field we persist. */
export function progressForBoardColumn(
  column: ConversationBoardColumn,
): ConversationProgress {
  switch (column) {
    case "backlog":
      return "todo";
    case "running":
      return "in_progress";
    case "needs_you":
      // Needs you is runtime attention; keep task progress as in_progress.
      return "in_progress";
    case "done":
      return "done";
  }
}

export function nextPriority(
  current: ConversationPriority | undefined,
): ConversationPriority | null {
  const idx = PRIORITY_CYCLE.findIndex((p) => p === (current ?? null));
  const next = PRIORITY_CYCLE[(idx + 1) % PRIORITY_CYCLE.length];
  return next;
}

export function nextProgress(
  current: ConversationProgress | undefined,
): ConversationProgress {
  const cur = current ?? "todo";
  const idx = PROGRESS_CYCLE.indexOf(cur);
  return PROGRESS_CYCLE[(idx + 1) % PROGRESS_CYCLE.length] ?? "todo";
}
