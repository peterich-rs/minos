/**
 * Pure mapping for daemon git activity payloads → UI model.
 * Kept free of `@/` imports so node:test can load it directly.
 */

import type { TimelineGitActivity } from "../../shared/lib/mock-data.ts";

type RawGitActivity = {
  kind?: string;
  branch?: string;
  worktreePath?: string;
  worktree_path?: string;
  baseBranch?: string;
  base_branch?: string;
  count?: number;
  subjects?: string[];
  head?: string;
  url?: string;
  number?: number;
  title?: string;
  summary?: string;
  mergeCommit?: string;
  merge_commit?: string;
};

export function normalizeGitActivity(
  raw: RawGitActivity | null | undefined,
): TimelineGitActivity | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const kind = raw.kind;
  if (
    kind !== "worktree_created" &&
    kind !== "commits_made" &&
    kind !== "pr_opened" &&
    kind !== "checks_failed" &&
    kind !== "ready_for_review" &&
    kind !== "merged"
  ) {
    return undefined;
  }
  return {
    kind,
    branch: raw.branch ?? undefined,
    worktreePath: raw.worktreePath ?? raw.worktree_path ?? undefined,
    baseBranch: raw.baseBranch ?? raw.base_branch ?? undefined,
    count: raw.count ?? undefined,
    subjects: raw.subjects ?? undefined,
    head: raw.head ?? undefined,
    url: raw.url ?? undefined,
    number: raw.number ?? undefined,
    title: raw.title ?? undefined,
    summary: raw.summary ?? undefined,
    mergeCommit: raw.mergeCommit ?? raw.merge_commit ?? undefined,
  };
}

export function timelineKindForMessage(
  kind: string,
  gitActivity: TimelineGitActivity | undefined,
): "text" | "tool_summary" | "approval" | "git_activity" {
  if (gitActivity || kind === "git_activity") return "git_activity";
  if (kind === "approval" || kind === "tool_summary") return kind;
  return "text";
}
