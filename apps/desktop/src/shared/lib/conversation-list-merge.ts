/**
 * Merge daemon project conversation rows with Hub digests for the rail.
 *
 * Gate: isHubImMode = authenticated + token (NOT host-linked).
 * When daemon is absent (auth without host / no local shell), still show
 * Hub-only rows with defaults for host-local fields.
 *
 * P1 unread SSOT:
 * - `unreadSource: "hub"` (Hub IM mode): unread only from Hub digest / live
 *   patch. Never use local `readMessageCountById` baseline dual-track.
 * - `unreadSource: "local"` (daemon-only / unauthenticated): row.unread from
 *   local baseline is authoritative.
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

/** Where rail unread badges come from (P1 single-track). */
export type UnreadSource = "hub" | "local";

function hubPreview(d: HubConversationDigest): string {
  return d.preview?.trim() || "No messages yet";
}

/**
 * Resolve unread for one row under P1 single-track rules.
 * Focused conversation always clears local badge (Hub mark-read is separate).
 */
export function resolveRailUnread(input: {
  conversationId: string;
  focusedConversationId?: string | null;
  unreadSource: UnreadSource;
  hubUnreadCount?: number | null;
  localUnread?: number | null;
}): number | undefined {
  const id = input.conversationId.trim();
  if (!id) return undefined;
  if (id === (input.focusedConversationId?.trim() ?? null)) {
    return undefined;
  }
  if (input.unreadSource === "hub") {
    const n = input.hubUnreadCount ?? 0;
    return n > 0 ? n : undefined;
  }
  const local = input.localUnread ?? 0;
  return local > 0 ? local : undefined;
}

/**
 * Merge daemon rows (project-scoped) with account-scoped Hub digests.
 * - title, preview, lastMessageAt → Hub preferred when present
 * - unread → see `unreadSource` (P1)
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
  /**
   * P1: `"hub"` when authenticated Hub IM mode (digest SSOT);
   * `"local"` for daemon-only / unauthenticated baseline.
   * Default `"hub"` when digests are supplied historically; callers in hub
   * mode must pass `"hub"` explicitly for single-track.
   */
  unreadSource?: UnreadSource;
}): Conversation[] {
  const {
    daemonRows,
    hubDigests,
    projectId,
    includeHubOnly = true,
    focusedConversationId = null,
    unreadSource = hubDigests.length > 0 ? "hub" : "local",
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
    const unread = resolveRailUnread({
      conversationId: id,
      focusedConversationId,
      unreadSource,
      hubUnreadCount: hub?.unreadCount,
      // Hub mode never falls back to local baseline (single-track).
      localUnread: unreadSource === "local" ? row.unread : undefined,
    });
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
        unread: resolveRailUnread({
          conversationId: id,
          focusedConversationId,
          unreadSource: "hub",
          hubUnreadCount: hub.unreadCount,
        }),
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
