/**
 * Project SessionEntity into membership list caches.
 *
 * After every Entity commit, list rows that already contain the sessionId must
 * refresh status fields from Entity. Attention queue:
 * - when ready → re-derive from sessionsById (no stale copies)
 * - otherwise → only update/remove existing rows (hydrate owns membership)
 */

import {
  entityNeedsAttention,
  projectSessionFromEntity,
  type SessionEntity,
} from "./session-entity.ts";

/** Minimal list-row shape (matches store ProjectSession). */
export type SessionListRow = {
  id: string;
  conversationId: string;
  conversationTitle?: string;
  agent: string;
  shortId: string;
  status: string;
  model: string;
  parentId?: string;
  summary: string;
  needsContinue?: boolean;
  firstTsMs?: number;
  lastTsMs?: number;
  messageCount?: number;
};

export type SessionListProjectionInput = {
  sessionsById: Record<string, SessionEntity>;
  sessionsByConversation: Record<string, SessionListRow[]>;
  projectSessionsByProject: Record<string, SessionListRow[]>;
  attentionSessions: SessionListRow[];
  /**
   * When true, Attention queue is re-derived from Entity after each projection
   * (avoids hydrate-only sibling lag once the page has loaded).
   */
  attentionReady?: boolean;
};

export type SessionListProjectionResult = {
  sessionsByConversation: Record<string, SessionListRow[]>;
  projectSessionsByProject: Record<string, SessionListRow[]>;
  attentionSessions: SessionListRow[];
};

function rowFromEntity(entity: SessionEntity): SessionListRow {
  return projectSessionFromEntity(entity) as SessionListRow;
}

/** Patch status fields on every list that already contains sessionId. */
function patchMembershipLists(
  lists: Record<string, SessionListRow[]>,
  sessionId: string,
  row: SessionListRow,
): Record<string, SessionListRow[]> {
  let changed = false;
  const next: Record<string, SessionListRow[]> = { ...lists };
  for (const [key, list] of Object.entries(lists)) {
    if (!list.some((x) => x.id === sessionId)) continue;
    next[key] = list.map((sess) =>
      sess.id === sessionId ? { ...sess, ...row } : sess,
    );
    changed = true;
  }
  return changed ? next : lists;
}

/**
 * Ensure the session appears under its conversation inspector list.
 *
 * Live manager events only know `sessionId` + status. Hydrate may have placed
 * the row already; if not (race after SessionAdded shell), invent membership
 * for `row.conversationId` so Running→Idle does not stay stuck until the user
 * remounts Sessions / Inspector.
 */
function upsertConversationMembership(
  lists: Record<string, SessionListRow[]>,
  sessionId: string,
  row: SessionListRow,
): Record<string, SessionListRow[]> {
  const patched = patchMembershipLists(lists, sessionId, row);
  const convId = row.conversationId?.trim();
  if (!convId) return patched;

  const current = patched[convId] ?? lists[convId] ?? [];
  if (current.some((x) => x.id === sessionId)) {
    // patchMembershipLists already rewrote the row when present.
    return patched;
  }
  return {
    ...patched,
    [convId]: [row, ...current],
  };
}

/**
 * Re-derive Attention queue from Entity map (sorted by lastTsMs desc).
 * Only entities that currently need attention appear.
 */
export function rederiveAttentionFromEntities(
  sessionsById: Record<string, SessionEntity>,
): SessionListRow[] {
  return Object.values(sessionsById)
    .filter(entityNeedsAttention)
    .map(rowFromEntity)
    .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0));
}

/**
 * Project one SessionEntity into Inspector / SessionList / Attention caches.
 * Entity must already be written to sessionsById before calling this.
 *
 * Inspector (`sessionsByConversation`): upserts when `conversationId` is known
 * so live SessionStateChanged can surface without a full re-list.
 * Project SessionList: membership patch only here (status refresh). New
 * project membership is upserted in `commitSessionEntity` once `projectId`
 * can be resolved from the conversation row — hydrate still owns full history.
 * Attention: re-derive when ready; else update/drop only.
 */
export function projectEntityIntoLists(
  s: SessionListProjectionInput,
  sessionId: string,
): SessionListProjectionResult {
  const entity = s.sessionsById[sessionId];
  if (!entity) {
    return {
      sessionsByConversation: s.sessionsByConversation,
      projectSessionsByProject: s.projectSessionsByProject,
      attentionSessions: s.attentionSessions,
    };
  }
  const row = rowFromEntity(entity);

  const sessionsByConversation = upsertConversationMembership(
    s.sessionsByConversation,
    sessionId,
    row,
  );
  const projectSessionsByProject = patchMembershipLists(
    s.projectSessionsByProject,
    sessionId,
    row,
  );

  let attentionSessions = s.attentionSessions;
  if (s.attentionReady) {
    attentionSessions = rederiveAttentionFromEntities(s.sessionsById);
  } else {
    const idx = attentionSessions.findIndex((x) => x.id === sessionId);
    if (entityNeedsAttention(entity)) {
      if (idx >= 0) {
        attentionSessions = attentionSessions.map((sess) =>
          sess.id === sessionId ? { ...sess, ...row } : sess,
        );
      }
      // Do not invent Attention rows when queue not ready — hydrate owns membership.
    } else if (idx >= 0) {
      attentionSessions = attentionSessions.filter((x) => x.id !== sessionId);
    }
  }

  return {
    sessionsByConversation,
    projectSessionsByProject,
    attentionSessions,
  };
}

/**
 * After bulk Entity upserts, project every updated sessionId into sibling lists.
 * Use from hydrate loaders so Inspector/SessionList/Attention stay in sync.
 */
export function projectSessionIdsIntoLists(
  s: SessionListProjectionInput,
  sessionIds: readonly string[],
): SessionListProjectionResult {
  let sessionsByConversation = s.sessionsByConversation;
  let projectSessionsByProject = s.projectSessionsByProject;
  let attentionSessions = s.attentionSessions;

  for (const sessionId of sessionIds) {
    const next = projectEntityIntoLists(
      {
        sessionsById: s.sessionsById,
        sessionsByConversation,
        projectSessionsByProject,
        attentionSessions,
        attentionReady: s.attentionReady,
      },
      sessionId,
    );
    sessionsByConversation = next.sessionsByConversation;
    projectSessionsByProject = next.projectSessionsByProject;
    attentionSessions = next.attentionSessions;
  }

  // One re-derive at end is enough when ready (avoids O(n²) full scans mid-loop).
  if (s.attentionReady) {
    attentionSessions = rederiveAttentionFromEntities(s.sessionsById);
  }

  return {
    sessionsByConversation,
    projectSessionsByProject,
    attentionSessions,
  };
}

/**
 * Build list rows from Entity after hydrate upserts (never raw DTO status alone).
 */
export function rowsFromEntities(
  sessionsById: Record<string, SessionEntity>,
  orderedIds: readonly string[],
): SessionListRow[] {
  return orderedIds.map((id) => {
    const entity = sessionsById[id];
    if (!entity) {
      return {
        id,
        conversationId: "",
        agent: "codex",
        shortId: id.slice(0, 8),
        status: "idle",
        model: "",
        summary: "",
      };
    }
    return rowFromEntity(entity);
  });
}

/**
 * Merge session rows into a project SessionList **membership** (upsert by id).
 * Used when Inspector hydrates a conversation's sessions — those must also
 * appear under Sessions tab for the same project (not only sessionsByConversation).
 */
export function mergeRowsIntoProjectSessionList(
  projectSessionsByProject: Record<string, SessionListRow[]>,
  projectId: string,
  rows: readonly SessionListRow[],
): Record<string, SessionListRow[]> {
  if (!projectId || rows.length === 0) return projectSessionsByProject;
  const prev = projectSessionsByProject[projectId] ?? [];
  const byId = new Map(prev.map((s) => [s.id, s]));
  for (const row of rows) {
    if (!row.id) continue;
    const existing = byId.get(row.id);
    byId.set(row.id, existing ? { ...existing, ...row } : row);
  }
  const merged = Array.from(byId.values()).sort(
    (a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0),
  );
  return {
    ...projectSessionsByProject,
    [projectId]: merged,
  };
}

/**
 * Resolve which project SessionList a conversation belongs to.
 * Prefer conversation.projectId; fall back to an existing project list that
 * already contains a row for this conversationId (weak signal).
 */
export function resolveProjectIdForConversation(
  conversationId: string,
  conversationProjectById: Readonly<Record<string, string>>,
  projectSessionsByProject: Readonly<Record<string, SessionListRow[]>>,
): string | undefined {
  const cid = conversationId.trim();
  if (!cid) return undefined;
  const fromRow = conversationProjectById[cid]?.trim();
  if (fromRow) return fromRow;
  for (const [projectId, rows] of Object.entries(projectSessionsByProject)) {
    if (rows.some((r) => r.conversationId === cid)) return projectId;
  }
  return undefined;
}
