/**
 * Attention inbox projection — human Home Feed for the desktop viewer.
 *
 * Pure: combines host facts (sessions needing action, conversation list fields)
 * with client read-state-derived unread into a skimable inbox. Does not own
 * persistence; callers pass already-computed `unread` on conversations.
 */

export type AttentionInboxCategory =
  | "approval"
  | "failed"
  | "suspended"
  | "unread";

export type AttentionInboxFilter = "all" | AttentionInboxCategory;

export type AttentionInboxSession = {
  id: string;
  conversationId: string;
  conversationTitle?: string;
  agent: string;
  shortId: string;
  status: string;
  summary: string;
  lastTsMs?: number;
};

export type AttentionInboxConversation = {
  id: string;
  projectId: string;
  title: string;
  preview: string;
  updatedAtMs: number;
  unread?: number;
  approvalCount?: number;
};

export type AttentionInboxProject = {
  id: string;
  name: string;
};

export type AttentionInboxItem = {
  /** Stable row key. */
  id: string;
  category: AttentionInboxCategory;
  conversationId: string;
  projectId: string;
  projectName: string;
  conversationTitle: string;
  sessionId?: string;
  agent?: string;
  shortId?: string;
  title: string;
  preview: string;
  updatedAtMs: number;
  unreadCount?: number;
};

export type BuildAttentionInboxInput = {
  conversations: AttentionInboxConversation[];
  projects: AttentionInboxProject[];
  sessions: AttentionInboxSession[];
};

const CATEGORY_ORDER: Record<AttentionInboxCategory, number> = {
  approval: 0,
  failed: 1,
  suspended: 2,
  unread: 3,
};

function projectName(
  projects: AttentionInboxProject[],
  projectId: string,
): string {
  return projects.find((p) => p.id === projectId)?.name ?? "—";
}

function sessionCategory(
  status: string,
): "approval" | "failed" | "suspended" | null {
  if (status === "needs_approval") return "approval";
  if (status === "failed") return "failed";
  if (status === "suspended") return "suspended";
  return null;
}

function sessionTitle(category: "approval" | "failed" | "suspended"): string {
  if (category === "approval") return "Approval required";
  if (category === "failed") return "Session failed";
  return "Session paused";
}

/**
 * Build a sorted Attention inbox.
 *
 * - Session rows: approval / failed / suspended (action-required runtime).
 * - Unread rows: conversations with unread > 0. When a conversation already
 *   has a session row, still emit unread so message backlog stays visible
 *   under the Unread filter; the All list shows both.
 */
export function buildAttentionInbox(
  input: BuildAttentionInboxInput,
): AttentionInboxItem[] {
  const { conversations, projects, sessions } = input;
  const convById = new Map(conversations.map((c) => [c.id, c]));
  const items: AttentionInboxItem[] = [];

  for (const session of sessions) {
    const category = sessionCategory(session.status);
    if (!category) continue;
    const conv = convById.get(session.conversationId);
    const projectId = conv?.projectId ?? "";
    items.push({
      id: `session:${session.id}`,
      category,
      conversationId: session.conversationId,
      projectId,
      projectName: projectName(projects, projectId),
      conversationTitle:
        conv?.title ?? session.conversationTitle ?? "Conversation",
      sessionId: session.id,
      agent: session.agent,
      shortId: session.shortId,
      title: sessionTitle(category),
      preview: session.summary || conv?.preview || "",
      updatedAtMs: session.lastTsMs ?? conv?.updatedAtMs ?? 0,
      unreadCount: conv?.unread,
    });
  }

  for (const conv of conversations) {
    const unread = conv.unread ?? 0;
    if (unread <= 0) continue;
    items.push({
      id: `unread:${conv.id}`,
      category: "unread",
      conversationId: conv.id,
      projectId: conv.projectId,
      projectName: projectName(projects, conv.projectId),
      conversationTitle: conv.title,
      title:
        unread === 1 ? "1 unread message" : `${unread} unread messages`,
      preview: conv.preview || "",
      updatedAtMs: conv.updatedAtMs ?? 0,
      unreadCount: unread,
    });
  }

  items.sort((a, b) => {
    if (b.updatedAtMs !== a.updatedAtMs) return b.updatedAtMs - a.updatedAtMs;
    return CATEGORY_ORDER[a.category] - CATEGORY_ORDER[b.category];
  });
  return items;
}

export function filterAttentionInbox(
  items: AttentionInboxItem[],
  filter: AttentionInboxFilter,
): AttentionInboxItem[] {
  if (filter === "all") return items;
  return items.filter((item) => item.category === filter);
}

export function countAttentionInboxByCategory(
  items: AttentionInboxItem[],
): Record<AttentionInboxFilter, number> {
  const counts: Record<AttentionInboxFilter, number> = {
    all: items.length,
    approval: 0,
    failed: 0,
    suspended: 0,
    unread: 0,
  };
  for (const item of items) {
    counts[item.category] += 1;
  }
  return counts;
}

/** Sidebar / project badge: unread messages + pending approvals. */
export function conversationAttentionScore(c: {
  unread?: number;
  approvalCount?: number;
}): number {
  return (c.unread ?? 0) + (c.approvalCount ?? 0);
}
