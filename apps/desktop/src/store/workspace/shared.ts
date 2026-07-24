/**
 * Cross-slice helpers used by multiple workspace action modules.
 */
import { daemonApi } from "@/shared/lib/daemon";
import type { KnownAgent } from "@/shared/lib/agent-route";
import type { WorkspaceGet } from "./types";

/**
 * After send/retry: always quiet-reload Timeline; quiet-reload Inspector only
 * when that working set already exists (never invent list_sessions on open).
 */
export async function quietRefreshConversationSlices(
  get: WorkspaceGet,
  conversationId: string,
): Promise<void> {
  await get().loadTimeline(conversationId, { quiet: true });
  const st = get();
  if (
    Object.prototype.hasOwnProperty.call(
      st.sessionsByConversation,
      conversationId,
    ) ||
    Object.prototype.hasOwnProperty.call(
      st.inspectorStatusByConversation,
      conversationId,
    )
  ) {
    await get().loadInspector(conversationId, { quiet: true });
  }
}

/**
 * Send/retry need sessions for @agent#short and reuse. Use-case hydrate when
 * the Inspector working set is missing — not a navigation dual-pack.
 */
export async function ensureSessionsForRouting(
  get: WorkspaceGet,
  conversationId: string,
): Promise<void> {
  if (
    Object.prototype.hasOwnProperty.call(
      get().sessionsByConversation,
      conversationId,
    )
  ) {
    return;
  }
  await get().loadInspector(conversationId, { quiet: true });
}

/**
 * Start a fresh agent session in a conversation.
 *
 * Profile binding is server-owned via `profile_id`:
 * - When `profileId` is provided (e.g. `@ProfileName` mention), pass it through.
 * - Bare `@agent` convenience: if no profileId, pick the **newest** host profile
 *   for that runtime and pass its id (not silent field copy). Documented product
 *   default — not a silent heuristic for explicit profile mentions.
 * - If no profiles exist, start with daemon defaults (no profile_id).
 */
export async function startNewAgentSession(
  conversationId: string,
  agent: KnownAgent,
  workspacePath: string,
  profileId?: string | null,
): Promise<string> {
  let resolvedProfileId = profileId?.trim() || undefined;
  if (!resolvedProfileId) {
    try {
      const { profiles } = await daemonApi.listAgentProfiles();
      const match = (profiles ?? [])
        .filter((p) => p.runtime_agent === agent)
        .sort((a, b) => (b.updated_at_ms ?? 0) - (a.updated_at_ms ?? 0))[0];
      if (match?.id) {
        resolvedProfileId = match.id;
      }
    } catch {
      /* profiles optional for bare @agent */
    }
  }
  const started = await daemonApi.startAgentInConversation(
    conversationId,
    agent,
    workspacePath,
    { profileId: resolvedProfileId },
  );
  return started.sessionId;
}
