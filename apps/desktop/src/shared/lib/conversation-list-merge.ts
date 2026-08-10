/**
 * Merge daemon project conversation rows with Hub digests for the rail.
 *
 * Gate: isCloudImMode = authenticated + token (NOT host-linked).
 * When daemon is absent (auth without host / no local shell), still show
 * Hub-only rows with defaults for host-local fields.
 *
 * P1 unread SSOT:
 * - `unreadSource: "hub"` (Hub IM mode): unread only from Hub digest / live
 *   patch. Never use local `readMessageCountById` baseline dual-track.
 * - `unreadSource: "local"` (daemon-only / unauthenticated): row.unread from
 *   local baseline is authoritative.
 *
 * Last activity time:
 * - `updatedAtMs = max(hub.lastMessageAtMs, daemon.updatedAtMs)` so local
 *   host_projection lag cannot pin the rail to a stale Hub digest.
 * - preview follows the newer of the two sources (not Hub-only when lagging).
 * - Display strings are **not** stored; UI formats `updatedAtMs` at render.
 */

import type { Conversation } from "./mock-data.ts";
import { runtimesOfBots } from "./mock-data.ts";
import type { CloudConversationDigest } from "./cloud-digest-cache.ts";
import { positiveMs } from "./rail-activity.ts";

export type DaemonListRow = {
  id: string;
  projectId: string;
  title: string;
  preview?: string;
  /** @deprecated Wire fallback only; prefer updatedAtMs. */
  updatedAt?: string;
  updatedAtMs?: number;
  messageCount?: number;
  unread?: number;
  agentSessionCount?: number;
  participatingBots?: Conversation["participatingBots"];
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

function hubPreview(d: CloudConversationDigest): string {
  return d.preview?.trim() || "No messages yet";
}

/**
 * Placeholder titles invented when Hub/local lack a real name.
 * Never let these override a concrete title from the other source.
 */
const PLACEHOLDER_TITLES = new Set([
  "",
  "conversation",
  "direct agent sessions",
]);

/** True when title is missing or a known placeholder (case-insensitive). */
export function isPlaceholderConversationTitle(
  title: string | null | undefined,
): boolean {
  const t = title?.trim() ?? "";
  return PLACEHOLDER_TITLES.has(t.toLowerCase());
}

/**
 * Pick display title: real Hub title wins (multi-end rename SSOT);
 * Hub placeholder must not clobber a real local/daemon title.
 */
export function resolveConversationTitle(input: {
  hubTitle?: string | null;
  daemonTitle?: string | null;
}): string {
  const hub = input.hubTitle?.trim() ?? "";
  const daemon = input.daemonTitle?.trim() ?? "";
  if (!isPlaceholderConversationTitle(hub)) return hub;
  if (!isPlaceholderConversationTitle(daemon)) return daemon;
  return hub || daemon || "Conversation";
}

/**
 * Last-activity ms: newer of Hub digest and local daemon row.
 * Either side may lag (Hub outbox vs local-only tool noise); max is correct
 * for sort + list clock.
 */
export function resolveLastActivityMs(
  hubLastMessageAtMs: number | null | undefined,
  daemonUpdatedAtMs: number | null | undefined,
): number {
  return Math.max(
    positiveMs(hubLastMessageAtMs),
    positiveMs(daemonUpdatedAtMs),
  );
}

/**
 * Preview text for the newer activity source. When timestamps tie, Hub wins
 * (multi-end rename/preview SSOT). When daemon is strictly newer, use local
 * preview so host_projection lag does not freeze the rail.
 */
export function resolveListPreview(input: {
  hub?: CloudConversationDigest | null;
  daemonPreview?: string | null;
  hubLastMessageAtMs?: number | null;
  daemonUpdatedAtMs?: number | null;
}): string {
  const hubMs = positiveMs(input.hubLastMessageAtMs);
  const daemonMs = positiveMs(input.daemonUpdatedAtMs);
  const hub = input.hub;
  if (hub && hubMs >= daemonMs) {
    return hubPreview(hub);
  }
  const local = input.daemonPreview?.trim();
  if (local) return local;
  if (hub) return hubPreview(hub);
  return "No messages yet";
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
 * - title → Hub preferred when present
 * - preview / last activity → newer of Hub vs daemon (see resolve*)
 * - unread → see `unreadSource` (P1)
 * - projectId, agents, git, priority, progress, running, approval → daemon
 * - Hub-only ids (no daemon row) still appear with omit/default host fields
 */
export function mergeConversationList(input: {
  daemonRows: readonly DaemonListRow[];
  hubDigests: readonly CloudConversationDigest[];
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
    const lastMs = resolveLastActivityMs(
      hub?.lastMessageAtMs,
      row.updatedAtMs,
    );
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
      title: resolveConversationTitle({
        hubTitle: hub?.title,
        daemonTitle: row.title,
      }),
      preview: resolveListPreview({
        hub,
        daemonPreview: row.preview,
        hubLastMessageAtMs: hub?.lastMessageAtMs,
        daemonUpdatedAtMs: row.updatedAtMs,
      }),
      updatedAtMs: lastMs,
      unread,
      messageCount: row.messageCount ?? 0,
      boardColumn: row.boardColumn ?? "backlog",
      agentSessionCount: row.agentSessionCount ?? 0,
      // participatingBots = roster SSOT; participatingAgents derived for host tokens.
      participatingBots: row.participatingBots ?? [],
      participatingAgents: (() => {
        const fromBots = runtimesOfBots(row.participatingBots);
        if (fromBots.length > 0) return fromBots;
        return (row.participatingAgents ?? [])
          .map((a) => a.trim().toLowerCase())
          .filter(Boolean);
      })(),
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
      const lastMs = resolveLastActivityMs(hub.lastMessageAtMs, 0);
      out.push({
        id,
        projectId: projectId || "",
        title: resolveConversationTitle({
          hubTitle: hub.title,
          daemonTitle: null,
        }),
        preview: hubPreview(hub),
        updatedAtMs: lastMs,
        unread: resolveRailUnread({
          conversationId: id,
          focusedConversationId,
          unreadSource: "hub",
          hubUnreadCount: hub.unreadCount,
        }),
        messageCount: 0,
        boardColumn: "backlog",
        agentSessionCount: 0,
        participatingBots: [],
        participatingAgents: [],
        runningCount: 0,
        approvalCount: 0,
      });
    }
  }

  out.sort((a, b) => (b.updatedAtMs || 0) - (a.updatedAtMs || 0));
  return out;
}
