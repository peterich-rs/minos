import type { TimelineMessage } from "@/shared/domain/collaboration";
import { agentMeta } from "@/shared/lib/mock-data";
import { shortSessionId, type KnownAgent } from "@/shared/lib/agent-route";

/** Truncate a worktree path for header meta chips. */
export function shortWorktree(path: string): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

/** Collapse reply parent body to a single-line preview. */
export function replyPreviewBody(body: string, maxChars = 120): string {
  const collapsed = body
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join(" ");
  if (collapsed.length <= maxChars) return collapsed;
  return `${collapsed.slice(0, maxChars - 1)}…`;
}

/** Author label for a reply-parent chip (You / System / Agent #short). */
export function replyAuthorLabel(parent: TimelineMessage): string {
  if (parent.role === "user") return "You";
  if (parent.role === "system") return "System";
  const agentKey = parent.agent as KnownAgent | undefined;
  const name =
    (agentKey && agentMeta[agentKey]?.label) || parent.agent || "Agent";
  if (parent.sessionId) {
    return `${name} #${shortSessionId(parent.sessionId)}`;
  }
  return name;
}
