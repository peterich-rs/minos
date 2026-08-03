/**
 * Merge daemon project conversation rows with Hub digests for the rail.
 *
 * Gate: isHubImMode = authenticated + token (NOT host-linked).
 * When daemon is absent (auth without host / no local shell), still show
 * Hub-only rows with defaults for host-local fields.
 */

import type { Conversation } from "./mock-data.ts";
import type { HubConversationDigest } from "./hub-digest-cache.ts";
import { formatRelative } from "./time.ts";

export type DaemonListRow = {
  id: string;
  projectId: string;
  title: string;
  preview?: string;
  updatedAt?: string;
  updatedAtMs?: number;
  messageCount?: number;
  unread?: number;
  agentSessionCount?: number;
  participatingAgents?: string[];
  runningCount?: number;
  approvalCount?: number;
  priority?: Conversation["priority"];
  progress?: Conversation["progress"];
  boardColumn?: Conversation["boardColumn"];
  branch?: string;
  worktree?: string;
  gitMode?: string;
  gitDirty?: boolean;
  gitHead?: string;
};

function hubPreview(d: HubConversationDigest): string {
  return d.preview?.trim() || "No messages yet";
}

/**
 * Merge daemon rows (project-scoped) with account-scoped Hub digests.
 * - title, preview, lastMessageAt, unread → Hub preferred when present
 * - projectId, agents, git, priority, progress, running, approval → daemon
 * - Hub-only ids (no daemon row) still appear with omit/default host fields
 */
export function mergeConversationList(input: {
  daemonRows: readonly DaemonListRow[];
  hubDigests: readonly HubConversationDigest[];
  projectId: string;
  /** When true, include Hub digests that have no matching daemon row. */
  includeHubOnly?: boolean;
  focusedConversationId?: string | null;
}): Conversation[] {
  const {
    daemonRows,
    hubDigests,
    projectId,
    includeHubOnly = true,
    focusedConversationId = null,
  } = input;

  const hubById = new Map(
    hubDigests.map((d) => [d.conversationId, d] as const),
  );
  const seen = new Set<string>();
  const out: Conversation[] = [];

  for (const row of daemonRows) {
    const id = row.id?.trim();
    if (!id) continue;
    seen.add(id);
    const hub = hubById.get(id);
    const lastMs = hub?.lastMessageAtMs || row.updatedAtMs || 0;
    const unread =
      id === focusedConversationId
        ? undefined
        : hub
          ? hub.unreadCount > 0
            ? hub.unreadCount
            : undefined
          : row.unread && row.unread > 0
            ? row.unread
            : undefined;
    out.push({
      id,
      projectId: row.projectId || projectId,
      title: hub?.title?.trim() || row.title || "Conversation",
      preview: hub ? hubPreview(hub) : row.preview || "No messages yet",
      updatedAt: lastMs ? formatRelative(lastMs) : (row.updatedAt ?? ""),
      updatedAtMs: lastMs,
      unread,
      messageCount: row.messageCount ?? 0,
      boardColumn: row.boardColumn ?? "todo",
      agentSessionCount: row.agentSessionCount ?? 0,
      participatingAgents: row.participatingAgents ?? [],
      runningCount: row.runningCount ?? 0,
      approvalCount: row.approvalCount ?? 0,
      priority: row.priority,
      progress: row.progress,
      branch: row.branch,
      worktree: row.worktree,
      gitMode: row.gitMode,
      gitDirty: row.gitDirty,
      gitHead: row.gitHead,
    });
  }

  if (includeHubOnly) {
    for (const hub of hubDigests) {
      const id = hub.conversationId;
      if (!id || seen.has(id)) continue;
      // Hub-only: no host shell — still show for multi-end IM inbox.
      const lastMs = hub.lastMessageAtMs || 0;
      out.push({
        id,
        projectId: projectId || "",
        title: hub.title?.trim() || "Conversation",
        preview: hubPreview(hub),
        updatedAt: lastMs ? formatRelative(lastMs) : "",
        updatedAtMs: lastMs,
        unread:
          id === focusedConversationId
            ? undefined
            : hub.unreadCount > 0
              ? hub.unreadCount
              : undefined,
        messageCount: 0,
        boardColumn: "todo",
        agentSessionCount: 0,
        participatingAgents: [],
        runningCount: 0,
        approvalCount: 0,
      });
    }
  }

  out.sort((a, b) => (b.updatedAtMs || 0) - (a.updatedAtMs || 0));
  return out;
}
