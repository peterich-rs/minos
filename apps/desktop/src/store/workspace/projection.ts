/**
 * SessionEntity commit + list projection glue for the workspace store.
 *
 * Sole write funnel: merge/patch Entity → commitSessionEntity →
 * membership lists + conversation running/approval aggregates from Entity Σ.
 */
import type { Conversation, Project } from "@/shared/lib/mock-data";
import type { SessionEntity } from "@/shared/lib/session-entity";
import {
  conversationAggregatesFromEntities,
  projectSessionFromEntity,
} from "@/shared/lib/session-entity";
import {
  mergeRowsIntoProjectSessionList,
  projectEntityIntoLists as projectEntityIntoListsPure,
  projectSessionIdsIntoLists,
  resolveProjectIdForConversation,
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
 * Recompute conversation.runningCount / approvalCount from Entity membership.
 * Optional project.needsAttention rollup when projects are present.
 */
export function recomputeConversationAggregates(
  conversations: Conversation[],
  sessionsById: Record<string, SessionEntity>,
  conversationIds: readonly string[],
  projects?: Project[],
): { conversations: Conversation[]; projects?: Project[] } {
  const unique = [
    ...new Set(conversationIds.map((id) => id.trim()).filter(Boolean)),
  ];
  if (unique.length === 0) {
    return { conversations, projects };
  }

  let nextConversations = conversations;
  const touchedProjectIds = new Set<string>();

  for (const convId of unique) {
    const { runningCount, approvalCount } = conversationAggregatesFromEntities(
      sessionsById,
      convId,
    );
    const row = nextConversations.find((c) => c.id === convId);
    if (
      row &&
      row.runningCount === runningCount &&
      row.approvalCount === approvalCount
    ) {
      continue;
    }
    nextConversations = patchLocalConversation(nextConversations, convId, {
      runningCount,
      approvalCount,
    });
    const projectId = nextConversations.find((c) => c.id === convId)?.projectId;
    if (projectId) touchedProjectIds.add(projectId);
  }

  let projectsOut = projects;
  if (projectsOut && touchedProjectIds.size > 0) {
    for (const projectId of touchedProjectIds) {
      projectsOut = patchProjectAggregates(
        projectsOut,
        projectId,
        nextConversations,
      );
    }
  }

  return {
    conversations: nextConversations,
    ...(projectsOut ? { projects: projectsOut } : {}),
  };
}

/**
 * Sole write path for SessionEntity + list projection + conversation aggregates.
 */
export function commitSessionEntity(
  s: SessionListSlice,
  entity: SessionEntity,
  _opts?: { elevateApprovalCount?: boolean },
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

  // Project SessionList hydrate still owns full project history, but live /
  // start-session commits must upsert membership immediately. Otherwise the
  // Sessions tab only shows the current conversation's runs after a delayed
  // list_project_sessions round-trip (or never, while keep-alive was stale).
  let projectSessionsByProject = lists.projectSessionsByProject;
  const conversationProjectById: Record<string, string> = {};
  for (const c of s.conversations) {
    if (c.projectId) conversationProjectById[c.id] = c.projectId;
  }
  const projectId = resolveProjectIdForConversation(
    entity.conversationId,
    conversationProjectById,
    projectSessionsByProject,
  );
  if (projectId) {
    projectSessionsByProject = mergeRowsIntoProjectSessionList(
      projectSessionsByProject,
      projectId,
      [projectSessionFromEntity(entity) as ProjectSession],
    ) as Record<string, ProjectSession[]>;
  }

  const convIds = [
    entity.conversationId,
    prev?.conversationId ?? "",
  ].filter(Boolean);

  const aggregates = recomputeConversationAggregates(
    s.conversations,
    sessionsById,
    convIds,
    s.projects,
  );

  return {
    sessionsById,
    sessionsByConversation: lists.sessionsByConversation,
    projectSessionsByProject,
    attentionSessions: lists.attentionSessions,
    conversations: aggregates.conversations,
    ...(aggregates.projects ? { projects: aggregates.projects } : {}),
  };
}

/**
 * After bulk Entity map replacement (hydrate), re-project lists + aggregates
 * for the touched conversation ids.
 */
export function commitHydratedSessionEntities(
  s: SessionListSlice,
  sessionsById: Record<string, SessionEntity>,
  orderedIds: readonly string[],
  opts?: { primaryConversationId?: string },
): {
  sessionsById: Record<string, SessionEntity>;
  sessionsByConversation: Record<string, ProjectSession[]>;
  projectSessionsByProject: Record<string, ProjectSession[]>;
  attentionSessions: ProjectSession[];
  conversations: Conversation[];
  projects?: Project[];
} {
  const sibling = projectHydratedEntities(s, sessionsById, orderedIds);
  const convIds = new Set<string>();
  if (opts?.primaryConversationId) {
    convIds.add(opts.primaryConversationId);
  }
  for (const id of orderedIds) {
    const e = sessionsById[id];
    if (e?.conversationId) convIds.add(e.conversationId);
  }
  const aggregates = recomputeConversationAggregates(
    s.conversations,
    sessionsById,
    [...convIds],
    s.projects,
  );
  return {
    sessionsById,
    ...sibling,
    conversations: aggregates.conversations,
    ...(aggregates.projects ? { projects: aggregates.projects } : {}),
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
