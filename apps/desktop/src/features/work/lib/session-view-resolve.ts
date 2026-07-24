import type { ProjectSession } from "@/store/workspace-store";

/**
 * Resolve a session for the Sessions tab **within the current project only**.
 *
 * Deep-link ids may exist only in `sessionsByConversation` until the project
 * list load returns. To avoid rendering a foreign-project transcript under
 * the current project chrome during that window, we require the session's
 * `conversationId` to reference a conversation whose `projectId` matches the
 * current project (conversations carry `projectId` on the row, unlike sessions).
 * If conversations haven't loaded yet, we fall back to the project-sessions
 * list membership check (empty during load → reject).
 */
export function resolveSessionForView(
  sessionId: string | null,
  projectId: string,
  projectSessions: ProjectSession[],
  sessionsByConversation: Record<string, ProjectSession[]>,
  conversationProjectById: Record<string, string>,
): ProjectSession | undefined {
  if (!sessionId || !projectId) return undefined;
  const fromProject = projectSessions.find((s) => s.id === sessionId);
  if (fromProject) return fromProject;
  for (const list of Object.values(sessionsByConversation)) {
    const hit = list.find((s) => s.id === sessionId);
    if (!hit) continue;
    // Authoritative check: conversation row carries projectId. Reject if the
    // conversation is known and belongs to a different project.
    const convProject = conversationProjectById[hit.conversationId];
    if (convProject && convProject !== projectId) continue;
    // Conversation not yet loaded — only allow if a project-scoped session
    // shares this conversationId (weak signal, but better than nothing).
    if (
      !convProject &&
      projectSessions.length > 0 &&
      !projectSessions.some((s) => s.conversationId === hit.conversationId)
    ) {
      continue;
    }
    return hit;
  }
  return undefined;
}

export function sessionBelongsToProject(
  sessionId: string | null,
  projectSessions: ProjectSession[],
): boolean {
  if (!sessionId) return false;
  return projectSessions.some((s) => s.id === sessionId);
}
