/**
 * Browser / offline mock workspace bundle + known-agent CLI fallback inventory.
 */
import {
  conversations as mockConversations,
  projects as mockProjects,
  timelineByConversation as mockTimeline,
  agentSessions as mockSessions,
} from "@/shared/lib/mock-data";
import type { TimelineMessage } from "@/shared/domain/collaboration";
import {
  entityNeedsAttention,
  mergeSessionEntity,
  projectSessionFromEntity,
  type SessionEntity,
} from "@/shared/lib/session-entity";
import type { ProjectSession, WorkspaceState } from "./types";

/**
 * Mock / offline CLI inventory shaped like daemon `list_clis`.
 * Capability flags mirror domain SSOT (Codex/Grok effort; all model selection).
 * Not a second capability table for production — daemon inventory replaces this.
 */
export const KNOWN_AGENTS_FALLBACK: {
  agent: string;
  displayName: string;
  installed: boolean;
  status: string;
  supportsModelSelection: boolean;
  supportsReasoningEffort: boolean;
}[] = [
  {
    agent: "codex",
    displayName: "Codex",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
  {
    agent: "claude",
    displayName: "Claude",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "gemini",
    displayName: "Gemini",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "opencode",
    displayName: "OpenCode",
    installed: false,
    status: "missing",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "grok",
    displayName: "Grok",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
];

export function mockBundle(): Pick<
  WorkspaceState,
  | "source"
  | "projects"
  | "conversations"
  | "messagesByConversation"
  | "sessionsByConversation"
  | "sessionsById"
  | "projectSessionsByProject"
  | "attentionSessions"
  | "attentionStatus"
> {
  const messagesByConversation: Record<string, TimelineMessage[]> = {
    ...mockTimeline,
  };
  const sessionsByConversation: Record<string, ProjectSession[]> = {};
  const sessionsById: Record<string, SessionEntity> = {};
  const projectSessionsByProject: Record<string, ProjectSession[]> = {};
  for (const s of mockSessions) {
    const list = sessionsByConversation[s.conversationId] ?? [];
    list.push(s);
    sessionsByConversation[s.conversationId] = list;
    const entity = mergeSessionEntity(undefined, {
      id: s.id,
      conversationId: s.conversationId,
      agent: s.agent,
      shortId: s.shortId,
      status: s.status,
      model: s.model,
      parentId: s.parentId,
      summary: s.summary,
      needsContinue: s.needsContinue,
    }, {
      pendingApproval: s.status === "needs_approval",
    });
    sessionsById[s.id] = entity;
    const projId =
      mockConversations.find((c) => c.id === s.conversationId)?.projectId ??
      "mock";
    const pList = projectSessionsByProject[projId] ?? [];
    pList.push(projectSessionFromEntity(entity) as ProjectSession);
    projectSessionsByProject[projId] = pList;
  }
  const attentionSessions = Object.values(sessionsById)
    .filter(entityNeedsAttention)
    .map((e) => projectSessionFromEntity(e) as ProjectSession);
  return {
    source: "mock",
    projects: mockProjects,
    conversations: mockConversations,
    messagesByConversation,
    sessionsByConversation,
    sessionsById,
    projectSessionsByProject,
    attentionSessions,
    attentionStatus: { phase: "ready", generation: 1 },
  };
}

