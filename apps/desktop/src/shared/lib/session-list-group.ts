import type {
  ProjectSession,
  SessionStatus,
} from "../domain/collaboration.ts";

export type ConversationSessionGroup = {
  conversationId: string;
  title: string;
  /** All sessions in this conversation (roots + subagents). */
  sessions: ProjectSession[];
  /** Top-level sessions only (`!parentId`). */
  roots: ProjectSession[];
  lastActivityMs: number;
  runningCount: number;
  attentionCount: number;
};

function isLiveStatus(status: SessionStatus): boolean {
  return status === "running" || status === "needs_approval";
}

/**
 * Group project sessions under conversations (Codex-style folder → runs).
 * Conversations ordered by most recent session activity DESC.
 * Roots within a conversation ordered by last activity DESC.
 */
export function groupSessionsByConversation(
  sessions: readonly ProjectSession[],
): ConversationSessionGroup[] {
  const byConv = new Map<string, ProjectSession[]>();
  for (const session of sessions) {
    const cid = session.conversationId || "unknown";
    const list = byConv.get(cid) ?? [];
    list.push(session);
    byConv.set(cid, list);
  }

  const groups: ConversationSessionGroup[] = [];
  for (const [conversationId, list] of byConv) {
    const title =
      list.find((s) => s.conversationTitle)?.conversationTitle?.trim() ||
      "Untitled conversation";
    const lastActivityMs = list.reduce(
      (max, s) => Math.max(max, s.lastTsMs ?? s.firstTsMs ?? 0),
      0,
    );
    const runningCount = list.filter((s) => isLiveStatus(s.status)).length;
    const attentionCount = list.filter(
      (s) => s.status === "needs_approval" || s.status === "suspended",
    ).length;
    const roots = list
      .filter((s) => !s.parentId)
      .sort(
        (a, b) =>
          (b.lastTsMs ?? b.firstTsMs ?? 0) - (a.lastTsMs ?? a.firstTsMs ?? 0),
      );
    groups.push({
      conversationId,
      title,
      sessions: list,
      roots,
      lastActivityMs,
      runningCount,
      attentionCount,
    });
  }

  groups.sort((a, b) => {
    if (a.lastActivityMs !== b.lastActivityMs) {
      return b.lastActivityMs - a.lastActivityMs;
    }
    return a.title.localeCompare(b.title);
  });
  return groups;
}

export function childrenOf(
  parentId: string,
  all: readonly ProjectSession[],
): ProjectSession[] {
  return all
    .filter((s) => s.parentId === parentId)
    .sort(
      (a, b) =>
        (b.lastTsMs ?? b.firstTsMs ?? 0) - (a.lastTsMs ?? a.firstTsMs ?? 0),
    );
}

/** Session is actively executing (show spinner). */
export function sessionIsExecuting(status: SessionStatus): boolean {
  return status === "running" || status === "needs_approval";
}

/**
 * Flatten conversation-folder session tree into virtual list rows.
 * Collapsed folders emit only a folder header; expanded emit folder + DFS session rows.
 */
export type VirtualSessionListRow =
  | {
      type: "folder";
      key: string;
      group: ConversationSessionGroup;
      collapsed: boolean;
    }
  | {
      type: "session";
      key: string;
      session: ProjectSession;
      all: ProjectSession[];
      depth: number;
      conversationId: string;
    }
  | {
      type: "empty-roots";
      key: string;
      conversationId: string;
    };

export function flattenSessionListRows(
  groups: readonly ConversationSessionGroup[],
  collapsedConvIds: ReadonlySet<string>,
): VirtualSessionListRow[] {
  const rows: VirtualSessionListRow[] = [];
  for (const group of groups) {
    const collapsed = collapsedConvIds.has(group.conversationId);
    rows.push({
      type: "folder",
      key: `folder:${group.conversationId}`,
      group,
      collapsed,
    });
    if (collapsed) continue;
    if (group.roots.length === 0) {
      rows.push({
        type: "empty-roots",
        key: `empty:${group.conversationId}`,
        conversationId: group.conversationId,
      });
      continue;
    }
    const walk = (session: ProjectSession, depth: number) => {
      rows.push({
        type: "session",
        key: `session:${session.id}`,
        session,
        all: group.sessions,
        depth,
        conversationId: group.conversationId,
      });
      for (const child of childrenOf(session.id, group.sessions)) {
        walk(child, depth + 1);
      }
    };
    for (const root of group.roots) {
      walk(root, 0);
    }
  }
  return rows;
}

/** Estimate row height for VirtualizedList (folder vs session). */
export function estimateSessionListRowSize(row: VirtualSessionListRow): number {
  if (row.type === "folder") return 40;
  if (row.type === "empty-roots") return 32;
  // Session row: avatar + status + optional summary line.
  return row.session.summary ? 76 : 58;
}
