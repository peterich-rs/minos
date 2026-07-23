/**
 * SessionEntity commit + list projection glue for the workspace store.
 */
import type { Conversation, Project } from "@/shared/lib/mock-data";
import type { SessionEntity } from "@/shared/lib/session-entity";
import { projectSessionFromEntity } from "@/shared/lib/session-entity";
import {
  projectEntityIntoLists as projectEntityIntoListsPure,
  projectSessionIdsIntoLists,
} from "@/shared/lib/session-list-projection";
import type {
  ProjectSession,
  SessionListSlice,
  WorkspaceGet,
  WorkspaceState,
} from "./types";
import { patchLocalConversation, patchProjectAggregates } from "./helpers";

/**
 * Project one SessionEntity into Inspector / SessionList / Attention caches.
 * Entity is already written to sessionsById before calling this.
 * When Attention queue is ready, re-derive membership from Entity.
 */
export function projectEntityIntoLists(
  s: SessionListSlice,
  sessionId: string,
): Pick<
  WorkspaceState,
  | "sessionsByConversation"
  | "projectSessionsByProject"
  | "attentionSessions"
> {
  const lists = projectEntityIntoListsPure(
    {
      sessionsById: s.sessionsById,
      sessionsByConversation: s.sessionsByConversation,
      projectSessionsByProject: s.projectSessionsByProject,
      attentionSessions: s.attentionSessions,
      attentionReady: s.attentionStatus?.phase === "ready",
    },
    sessionId,
  );
  return {
    sessionsByConversation:
      lists.sessionsByConversation as Record<string, ProjectSession[]>,
    projectSessionsByProject:
      lists.projectSessionsByProject as Record<string, ProjectSession[]>,
    attentionSessions: lists.attentionSessions as ProjectSession[],
  };
}

/**
 * After bulk Entity upserts on hydrate, refresh sibling list caches that already
 * contain those session ids (and re-derive Attention when ready).
 */
export function projectHydratedEntities(
  s: SessionListSlice,
  sessionsById: Record<string, SessionEntity>,
  sessionIds: readonly string[],
): Pick<
  WorkspaceState,
  | "sessionsByConversation"
  | "projectSessionsByProject"
  | "attentionSessions"
> {
  const lists = projectSessionIdsIntoLists(
    {
      sessionsById,
      sessionsByConversation: s.sessionsByConversation,
      projectSessionsByProject: s.projectSessionsByProject,
      attentionSessions: s.attentionSessions,
      attentionReady: s.attentionStatus?.phase === "ready",
    },
    sessionIds,
  );
  return {
    sessionsByConversation:
      lists.sessionsByConversation as Record<string, ProjectSession[]>,
    projectSessionsByProject:
      lists.projectSessionsByProject as Record<string, ProjectSession[]>,
    attentionSessions: lists.attentionSessions as ProjectSession[],
  };
}

/**
 * Sole write path for SessionEntity + list projection.
 * Optionally bumps conversation.approvalCount (+ project.needsAttention) when
 * elevating pending approval (§6.5 / §9 Board via list aggregates).
 */
export function commitSessionEntity(
  s: SessionListSlice,
  entity: SessionEntity,
  opts?: { elevateApprovalCount?: boolean },
): {
  sessionsById: Record<string, SessionEntity>;
  sessionsByConversation: Record<string, ProjectSession[]>;
  projectSessionsByProject: Record<string, ProjectSession[]>;
  attentionSessions: ProjectSession[];
  conversations: Conversation[];
  projects?: Project[];
} {
  const prev = s.sessionsById[entity.sessionId];
  const sessionsById = {
    ...s.sessionsById,
    [entity.sessionId]: entity,
  };
  const lists = projectEntityIntoLists(
    { ...s, sessionsById },
    entity.sessionId,
  );

  let conversations = s.conversations;
  let projectsOut: Project[] | undefined;
  const elevated =
    entity.hasPendingApproval &&
    (!prev || !prev.hasPendingApproval || prev.status !== "needs_approval");
  if (opts?.elevateApprovalCount !== false && elevated && entity.conversationId) {
    const convId = entity.conversationId;
    conversations = patchLocalConversation(s.conversations, convId, {
      approvalCount: Math.max(
        s.conversations.find((c) => c.id === convId)?.approvalCount ?? 0,
        1,
      ),
    });
    const projectId = conversations.find((c) => c.id === convId)?.projectId;
    if (projectId && s.projects) {
      projectsOut = patchProjectAggregates(s.projects, projectId, conversations);
    }
  }

  return {
    sessionsById,
    ...lists,
    conversations,
    ...(projectsOut ? { projects: projectsOut } : {}),
  };
}

/** Look up session metadata from Entity first, then list caches. */
export function findSessionRow(
  s: {
    sessionsById: Record<string, SessionEntity>;
    sessionsByConversation: Record<string, ProjectSession[]>;
    projectSessionsByProject: Record<string, ProjectSession[]>;
  },
  sessionId: string,
): ProjectSession | undefined {
  const listRow =
    Object.values(s.projectSessionsByProject)
      .flat()
      .find((x) => x.id === sessionId) ??
    Object.values(s.sessionsByConversation)
      .flat()
      .find((x) => x.id === sessionId);
  const entity = s.sessionsById[sessionId];
  if (!entity) return listRow;
  const fromEntity = projectSessionFromEntity(entity) as ProjectSession;
  // Shell entities (sessionAdded) often lack last_seq; prefer list hydrate
  // messageCount so transcript tail seek is not stuck at seq 0 forever.
  if (
    listRow &&
    (fromEntity.messageCount ?? 0) < (listRow.messageCount ?? 0)
  ) {
    return {
      ...fromEntity,
      messageCount: listRow.messageCount,
      firstTsMs: fromEntity.firstTsMs ?? listRow.firstTsMs,
      lastTsMs: fromEntity.lastTsMs ?? listRow.lastTsMs,
      conversationTitle:
        fromEntity.conversationTitle ?? listRow.conversationTitle,
    };
  }
  return fromEntity;
}

/** Bounded-concurrency map (badge quiet ConversationList hydrate). */
export async function mapPool<T>(
  items: readonly T[],
  concurrency: number,
  fn: (item: T) => Promise<void>,
): Promise<void> {
  if (items.length === 0) return;
  let next = 0;
  const workers = Array.from(
    { length: Math.min(concurrency, items.length) },
    async () => {
      while (next < items.length) {
        const idx = next++;
        await fn(items[idx]!);
      }
    },
  );
  await Promise.all(workers);
}

/**
 * Quiet-hydrate ConversationList for every known project so §6.5 badge /
 * project.needsAttention cover the full project index (daemon approvalCount
 * on conversation rows). Does not open Attention queue or keep project sessions.
 */
export async function quietHydrateAllConversationLists(
  get: WorkspaceGet,
): Promise<void> {
  if (get().source !== "daemon") return;
  const ids = get().projects.map((p) => p.id).filter(Boolean);
  await mapPool(ids, 4, async (projectId) => {
    try {
      await get().loadConversations(projectId, { quiet: true });
    } catch {
      /* per-project best-effort */
    }
  });
}
