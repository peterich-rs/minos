import {
  CheckCircle2,
  ExternalLink,
  GitBranch,
  GitCommit,
  GitMerge,
  GitPullRequest,
  Layers,
  XCircle,
} from "lucide-react";
import type { TimelineGitActivity } from "@/shared/lib/mock-data";
import { cn } from "@/shared/lib/utils";
import { shortWorktree } from "./lib/format";

function cardChrome(kind: TimelineGitActivity["kind"]): {
  icon: typeof GitBranch;
  accent: string;
  label: string;
} {
  switch (kind) {
    case "worktree_created":
      return {
        icon: Layers,
        accent: "border-sky-500/25 bg-sky-500/5 text-sky-800 dark:text-sky-200",
        label: "Worktree",
      };
    case "commits_made":
      return {
        icon: GitCommit,
        accent:
          "border-violet-500/25 bg-violet-500/5 text-violet-800 dark:text-violet-200",
        label: "Commits",
      };
    case "pr_opened":
      return {
        icon: GitPullRequest,
        accent:
          "border-emerald-500/25 bg-emerald-500/5 text-emerald-800 dark:text-emerald-200",
        label: "Pull request",
      };
    case "ready_for_review":
      return {
        icon: CheckCircle2,
        accent:
          "border-teal-500/25 bg-teal-500/5 text-teal-800 dark:text-teal-200",
        label: "Ready for review",
      };
    case "checks_failed":
      return {
        icon: XCircle,
        accent: "border-rose-500/25 bg-rose-500/5 text-rose-800 dark:text-rose-200",
        label: "Checks failed",
      };
    case "merged":
      return {
        icon: GitMerge,
        accent:
          "border-indigo-500/25 bg-indigo-500/5 text-indigo-800 dark:text-indigo-200",
        label: "Merged",
      };
  }
}

function primaryLine(activity: TimelineGitActivity): string {
  switch (activity.kind) {
    case "worktree_created":
      return activity.branch
        ? `Isolated branch \`${activity.branch}\``
        : "Isolated worktree created";
    case "commits_made": {
      const n = activity.count ?? activity.subjects?.length ?? 0;
      const first = activity.subjects?.[0];
      return first
        ? `${n} commit${n === 1 ? "" : "s"} · ${first}`
        : `${n} commit${n === 1 ? "" : "s"}`;
    }
    case "pr_opened":
      return activity.title
        ? activity.number
          ? `#${activity.number} · ${activity.title}`
          : activity.title
        : activity.number
          ? `Pull request #${activity.number}`
          : "Pull request opened";
    case "ready_for_review":
      return activity.branch
        ? `\`${activity.branch}\` is ready for review`
        : "Ready for review";
    case "checks_failed":
      return activity.summary || "Checks failed";
    case "merged":
      return activity.branch
        ? `Merged \`${activity.branch}\``
        : activity.mergeCommit
          ? `Merged · ${activity.mergeCommit.slice(0, 7)}`
          : "Merged";
  }
}

export function GitActivityCard({
  activity,
  time,
  className,
}: {
  activity: TimelineGitActivity;
  time?: string;
  className?: string;
}) {
  const chrome = cardChrome(activity.kind);
  const Icon = chrome.icon;
  const detailBits: string[] = [];
  if (activity.kind === "worktree_created") {
    if (activity.baseBranch) detailBits.push(`from ${activity.baseBranch}`);
    if (activity.worktreePath) {
      detailBits.push(shortWorktree(activity.worktreePath));
    }
  }
  if (activity.kind === "commits_made" && activity.head) {
    detailBits.push(activity.head.slice(0, 7));
  }
  if (activity.kind === "ready_for_review" && activity.head) {
    detailBits.push(activity.head.slice(0, 7));
  }
  if (activity.kind === "merged" && activity.mergeCommit) {
    detailBits.push(activity.mergeCommit.slice(0, 7));
  }

  return (
    <div
      className={cn(
        "flex w-full gap-3 rounded-xl border px-3 py-2.5 text-left",
        chrome.accent,
        className,
      )}
      data-testid="git-activity-card"
      data-git-kind={activity.kind}
    >
      <div className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-surface/70">
        <Icon className="h-3.5 w-3.5" aria-hidden />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className="text-2xs font-semibold uppercase tracking-wide opacity-80">
            {chrome.label}
          </span>
          {activity.branch && activity.kind !== "worktree_created" ? (
            <span className="inline-flex max-w-full items-center gap-1 truncate text-2xs opacity-80">
              <GitBranch className="h-3 w-3 shrink-0" />
              <span className="truncate">{activity.branch}</span>
            </span>
          ) : null}
          {time ? (
            <span className="ml-auto shrink-0 text-2xs opacity-60">{time}</span>
          ) : null}
        </div>
        <p className="mt-0.5 text-sm font-medium leading-snug text-ink">
          {primaryLine(activity)}
        </p>
        {detailBits.length > 0 ? (
          <p className="mt-0.5 truncate text-2xs opacity-75">
            {detailBits.join(" · ")}
          </p>
        ) : null}
        {activity.kind === "pr_opened" && activity.url ? (
          <a
            href={activity.url}
            target="_blank"
            rel="noreferrer"
            className="mt-1.5 inline-flex items-center gap-1 text-xs font-medium text-ink underline-offset-2 hover:underline"
          >
            Open pull request
            <ExternalLink className="h-3 w-3" />
          </a>
        ) : null}
        {activity.kind === "commits_made" &&
        activity.subjects &&
        activity.subjects.length > 1 ? (
          <ul className="mt-1.5 space-y-0.5 text-2xs opacity-80">
            {activity.subjects.slice(1, 4).map((subject) => (
              <li key={subject} className="truncate">
                · {subject}
              </li>
            ))}
            {activity.subjects.length > 4 ? (
              <li className="opacity-70">
                · +{activity.subjects.length - 4} more
              </li>
            ) : null}
          </ul>
        ) : null}
      </div>
    </div>
  );
}
